import SwiftUI
import WatchKit

extension Color {
  /// NyxID warm violet (`DESIGN.md` primary `#9775fa`), used sparingly.
  static let nyxAccent = Color(red: 151 / 255, green: 117 / 255, blue: 250 / 255)
  /// Row and control surface on the watch's black.
  static let nyxSurface = Color(white: 0.13)
  static let nyxDeny = Color.red
}

/// A round icon button: dark by default, white for the main action.
struct CircleButtonStyle: ButtonStyle {
  var primary = false
  var size: CGFloat = 34

  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .font(.system(size: size * 0.42, weight: .semibold))
      .foregroundStyle(primary ? Color.black : Color.white)
      .frame(width: size, height: size)
      .background(Circle().fill(primary ? Color.white : Color.nyxSurface))
      .opacity(configuration.isPressed ? 0.7 : 1)
  }
}

/// A capsule button: white for the affirmative choice, dark otherwise.
struct PillButtonStyle: ButtonStyle {
  var primary = false

  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .font(.footnote.weight(.semibold))
      .foregroundStyle(primary ? Color.black : Color.white)
      .padding(.horizontal, 14)
      .padding(.vertical, 8)
      .background(Capsule().fill(primary ? Color.white : Color.nyxSurface))
      .opacity(configuration.isPressed ? 0.7 : 1)
  }
}

/// Header of the three pages: settings, the page name, and the mic.
struct PageHeader: View {
  let title: String
  @Environment(WatchAppModel.self) private var model

  var body: some View {
    HStack {
      Button {
        model.showingSettings = true
      } label: {
        Image(systemName: "gearshape.fill")
      }
      .buttonStyle(CircleButtonStyle())
      .accessibilityLabel("Settings")
      Spacer(minLength: 4)
      Text(title)
        .font(.headline)
        .lineLimit(1)
        .accessibilityAddTraits(.isHeader)
      Spacer(minLength: 4)
      Button {
        model.ask(agentId: nil)
      } label: {
        Image(systemName: "mic.fill")
      }
      .buttonStyle(CircleButtonStyle(primary: true))
      .accessibilityLabel("Ask")
    }
    // Room on the right for the page dots beside the mic.
    .padding(.trailing, 10)
    .listRowBackground(Color.clear)
    .listRowInsets(EdgeInsets(top: 0, leading: 2, bottom: 4, trailing: 2))
  }
}

/// Back button plus whatever names the screen (an agent, a caption), for
/// the system top bar beside the clock: `.topBar { … }`.
struct TopBar<Title: View>: View {
  @Environment(\.dismiss) private var dismiss
  @ViewBuilder let title: Title

  var body: some View {
    HStack(spacing: 6) {
      Button {
        dismiss()
      } label: {
        Image(systemName: "chevron.left")
      }
      .buttonStyle(CircleButtonStyle(size: 28))
      .accessibilityLabel("Back")
      // Wider than this and watchOS drops the clock from the top bar, so
      // long names truncate instead.
      HStack(spacing: 6) {
        title
      }
      .lineLimit(1)
      .minimumScaleFactor(0.7)
      .frame(maxWidth: Self.titleWidth, alignment: .leading)
      .fixedSize(horizontal: false, vertical: true)
    }
  }

  /// The screen minus the back button and the clock (about 117pt).
  private static var titleWidth: CGFloat {
    max(56, WKInterfaceDevice.current().screenBounds.width - 117)
  }
}

extension View {
  /// For a bar of buttons pinned under scrolling content: content fades out
  /// above the bar, and the bar sits low, where the design puts it.
  func bottomBarBackdrop() -> some View {
    padding(.top, 16)
      .padding(.bottom, -14)
      .background(
        LinearGradient(
          stops: [.init(color: .black.opacity(0), location: 0), .init(color: .black, location: 0.4)],
          startPoint: .top, endPoint: .bottom)
        .ignoresSafeArea(edges: .bottom))
  }

  /// Puts back + title in the top bar. Scrolling screens otherwise keep an
  /// empty title band, which wastes a fifth of the screen.
  func topBar<Title: View>(@ViewBuilder _ title: @escaping () -> Title) -> some View {
    toolbar {
      ToolbarItem(placement: .topBarLeading) {
        TopBar(title: title)
      }
    }
  }
}

/// An agent's avatar, as on the iPhone and web: NyxBot wears the NyxID
/// mark; other agents get initials in a muted colour chosen from their id.
struct AgentBadge: View {
  let id: String
  let name: String
  var isNyxBot = false
  var size: CGFloat = 32

  var body: some View {
    let tint = AgentAvatarModel.tint(for: id, isNyxBot: isNyxBot)
    ZStack {
      Circle().fill(tint.fill.color)
      Circle().strokeBorder(tint.border.color, lineWidth: 1)
      if isNyxBot {
        Image("NyxMark")
          .resizable()
          .scaledToFit()
          .frame(width: size * 0.52, height: size * 0.52)
      } else {
        Text(AgentAvatarModel.initials(name))
          .font(.system(size: size * 0.34, weight: .bold))
          .foregroundStyle(tint.text.color)
      }
    }
    .frame(width: size, height: size)
    .accessibilityHidden(true)
  }
}

extension AgentAvatarModel.RGBA {
  var color: Color {
    Color(.sRGB, red: red, green: green, blue: blue, opacity: alpha)
  }
}

/// Avatar, a bold line and a grey line: the row used across the pages.
struct ListRow<Leading: View>: View {
  let title: String
  let subtitle: String?
  var unread = false
  @ViewBuilder let leading: Leading

  var body: some View {
    HStack(spacing: 10) {
      leading
      VStack(alignment: .leading, spacing: 1) {
        Text(title)
          .font(.body.weight(.semibold))
          .lineLimit(1)
        if let subtitle, !subtitle.isEmpty {
          Text(subtitle)
            .font(.footnote)
            .foregroundStyle(.secondary)
            .lineLimit(1)
        }
      }
      Spacer(minLength: 0)
      if unread {
        Circle()
          .fill(Color.nyxAccent)
          .frame(width: 7, height: 7)
          .accessibilityLabel("Unread")
      }
    }
    .padding(.vertical, 4)
    .accessibilityElement(children: .combine)
  }
}

/// A loading problem with a retry, in a couple of words.
struct ProblemRow: View {
  let message: String
  let retry: () -> Void

  var body: some View {
    Button(action: retry) {
      Label(message, systemImage: "arrow.clockwise")
        .font(.footnote)
        .foregroundStyle(.orange)
    }
  }
}

/// Three dots that pulse while an agent works.
struct TypingDots: View {
  @State private var phase = 0

  var body: some View {
    HStack(spacing: 4) {
      ForEach(0..<3) { index in
        Circle()
          .fill(Color.nyxAccent)
          .frame(width: 6, height: 6)
          .opacity(phase == index ? 1 : 0.35)
      }
    }
    .task {
      while !Task.isCancelled {
        try? await Task.sleep(nanoseconds: 350_000_000)
        phase = (phase + 1) % 3
      }
    }
    .accessibilityLabel("Working")
  }
}
