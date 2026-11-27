import SwiftUI
import WatchKit

/// Approvals waiting for the person, one card at a time: swipe or tap
/// Deny / Allow, see the result, then the next one.
struct ApprovalQueueView: View {
  let startAt: String?

  @Environment(WatchAppModel.self) private var model
  @Environment(\.dismiss) private var dismiss
  @State private var queue: [ApprovalRequest] = []
  @State private var index = 0
  @State private var grantDays: Int?
  @State private var loaded = false
  @State private var problem: String?
  @State private var deciding = false
  @State private var result: Outcome?
  @State private var dragOffset: CGFloat = 0
  // One key per request, so a repeated tap or swipe can't decide twice.
  @State private var decisionKeys: [String: String] = [:]

  struct Outcome: Equatable {
    let allowed: Bool
    let detail: String
  }

  private var current: ApprovalRequest? {
    queue.indices.contains(index) ? queue[index] : nil
  }

  var body: some View {
    Group {
      if let result {
        ResultView(outcome: result, hasNext: index + 1 < queue.count) {
          advance()
        } done: {
          dismiss()
        }
      } else if let current {
        cardScreen(current)
      } else if loaded {
        Text(problem ?? "Nothing waiting")
          .font(.footnote)
          .foregroundStyle(problem == nil ? Color.secondary : Color.orange)
      } else {
        ProgressView()
      }
    }
    .task { await load() }
  }

  private func cardScreen(_ approval: ApprovalRequest) -> some View {
    ScrollView {
      VStack(spacing: 6) {
        ApprovalCard(approval: approval, duration: approval.durationLabel(grantDays: grantDays))
          .offset(x: dragOffset)
          .rotationEffect(.degrees(Double(dragOffset) / 18))
          .gesture(
            DragGesture(minimumDistance: 12)
              .onChanged { value in
                guard approval.canDecideOnWatch, !deciding else { return }
                dragOffset = value.translation.width
              }
              .onEnded { value in
                guard approval.canDecideOnWatch, !deciding else { return }
                if value.translation.width > 60 {
                  Task { await decide(approval, allow: true) }
                } else if value.translation.width < -60 {
                  Task { await decide(approval, allow: false) }
                } else {
                  withAnimation(.spring) { dragOffset = 0 }
                }
              })
        if let problem {
          Text(problem)
            .font(.caption2)
            .foregroundStyle(.orange)
        }
      }
      .padding(.bottom, 44)
    }
    .safeAreaInset(edge: .bottom, spacing: 0) {
      decisionBar(approval)
    }
    .topBar {
      // Short enough to leave the clock in the top bar.
      Text("\(index + 1) OF \(queue.count)")
        .font(.caption2.weight(.semibold))
        .foregroundStyle(.secondary)
        .kerning(0.6)
        .accessibilityLabel("Approval \(index + 1) of \(queue.count)")
    }
  }

  @ViewBuilder
  private func decisionBar(_ approval: ApprovalRequest) -> some View {
    Group {
      if approval.canDecideOnWatch {
        HStack(spacing: 8) {
          Button {
            Task { await decide(approval, allow: false) }
          } label: {
            Label("Deny", systemImage: "xmark")
              .frame(maxWidth: .infinity)
          }
          .buttonStyle(PillButtonStyle())
          Button {
            Task { await decide(approval, allow: true) }
          } label: {
            Label("Allow", systemImage: "checkmark")
              .frame(maxWidth: .infinity)
          }
          .buttonStyle(PillButtonStyle(primary: true))
        }
        .disabled(deciding)
      } else {
        Label(approval.reviewCue, systemImage: approval.requiresOwnerChatReview ? "message" : "iphone")
          .font(.footnote)
          .foregroundStyle(.secondary)
      }
    }
    .bottomBarBackdrop()
  }

  private func load() async {
    guard !loaded else { return }
    do {
      var pending = try await model.service.pendingApprovals()
      if let startAt, !pending.contains(where: { $0.id == startAt }),
        let one = try? await model.service.approval(id: startAt), one.status == "pending"
      {
        pending.insert(one, at: 0)
      }
      queue = pending
      index = startAt.flatMap { id in pending.firstIndex { $0.id == id } } ?? 0
      grantDays = await model.service.grantExpiryDays()
      loaded = true
      if let queued = startAt.flatMap(model.takeQueuedDecision(for:)),
        let approval = current, approval.canDecideOnWatch
      {
        await decide(approval, allow: queued.approve)
      }
    } catch let failure as WatchAPIError {
      problem = failure.displayMessage
      loaded = true
    } catch {
      problem = "Couldn't load"
      loaded = true
    }
  }

  private func decide(_ approval: ApprovalRequest, allow: Bool) async {
    guard approval.canDecideOnWatch, !deciding else { return }
    deciding = true
    defer { deciding = false }
    let key = decisionKeys[approval.id] ?? WatchService.decisionKey(for: approval.id)
    decisionKeys[approval.id] = key
    withAnimation(.easeOut(duration: 0.2)) { dragOffset = allow ? 220 : -220 }
    do {
      _ = try await model.service.decide(approval, approve: allow, idempotencyKey: key)
      WKInterfaceDevice.current().play(allow ? .success : .stop)
      let duration = approval.durationLabel(grantDays: grantDays).lowercased()
      let span = duration == "once" ? "once" : "for \(duration)"
      problem = nil
      result = Outcome(
        allowed: allow,
        detail: allow
          ? "\(approval.title), \(approval.accessLabel.lowercased()), \(span)."
          : "\(approval.title) won't be used.")
    } catch let failure as WatchAPIError {
      withAnimation(.spring) { dragOffset = 0 }
      problem = failure.displayMessage
      WKInterfaceDevice.current().play(.failure)
    } catch {
      withAnimation(.spring) { dragOffset = 0 }
      problem = "Review on iPhone"
    }
  }

  private func advance() {
    result = nil
    dragOffset = 0
    index += 1
  }
}

/// The request itself: what, who, and how much access.
private struct ApprovalCard: View {
  let approval: ApprovalRequest
  let duration: String

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      Text(approval.headline)
        .font(.headline)
        .lineLimit(2)
        .fixedSize(horizontal: false, vertical: true)
      Text(approval.askedBy)
        .font(.footnote)
        .foregroundStyle(.secondary)
        .lineLimit(2)
        .fixedSize(horizontal: false, vertical: true)
      HStack(spacing: 6) {
        Chip(text: approval.accessLabel, warning: approval.risk == .high)
        Chip(text: duration)
      }
      if approval.canDecideOnWatch {
        HStack {
          Text("‹ DENY")
          Spacer()
          Text("ALLOW ›")
        }
        .font(.system(size: 9, weight: .semibold))
        .foregroundStyle(.tertiary)
        .kerning(0.6)
        .padding(.top, 2)
        .accessibilityHidden(true)
      } else if approval.requiresOwnerChatReview {
        Text(approval.reviewCue)
          .font(.footnote)
          .foregroundStyle(.secondary)
      }
    }
    .padding(10)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Color.nyxSurface))
    .accessibilityElement(children: .combine)
  }
}

private struct Chip: View {
  let text: String
  var warning = false

  var body: some View {
    Text(text)
      .font(.caption2.weight(.medium))
      .lineLimit(1)
      .minimumScaleFactor(0.8)
      .foregroundStyle(warning ? Color.red : Color.white)
      .padding(.horizontal, 7)
      .padding(.vertical, 3)
      .background(Capsule().fill(Color.white.opacity(0.08)))
  }
}

/// "Allowed · Gmail, read-only, once." then the next approval.
private struct ResultView: View {
  let outcome: ApprovalQueueView.Outcome
  let hasNext: Bool
  let next: () -> Void
  let done: () -> Void

  var body: some View {
    VStack(spacing: 6) {
      Spacer(minLength: 0)
      ZStack {
        Circle()
          .strokeBorder(Color.white, lineWidth: 2)
          .frame(width: 46, height: 46)
        Image(systemName: outcome.allowed ? "checkmark" : "xmark")
          .font(.system(size: 20, weight: .semibold))
      }
      .accessibilityHidden(true)
      Text(outcome.allowed ? "Allowed" : "Denied")
        .font(.headline)
      Text(outcome.detail)
        .font(.footnote)
        .foregroundStyle(.secondary)
        .multilineTextAlignment(.center)
        .lineLimit(2)
        .fixedSize(horizontal: false, vertical: true)
      Spacer(minLength: 0)
      Button(action: hasNext ? next : done) {
        HStack(spacing: 4) {
          Text(hasNext ? "Next approval" : "Done")
          if hasNext { Image(systemName: "chevron.right") }
        }
      }
      .buttonStyle(PillButtonStyle())
    }
    .frame(maxWidth: .infinity)
  }
}
