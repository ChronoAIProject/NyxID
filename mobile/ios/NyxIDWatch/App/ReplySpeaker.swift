import AVFoundation
import Foundation

/// Reads agent replies aloud with the watch's built-in speech voices.
@MainActor
final class ReplySpeaker: NSObject, AVSpeechSynthesizerDelegate {
  static let shared = ReplySpeaker()

  private let synthesizer = AVSpeechSynthesizer()
  private(set) var isSpeaking = false
  var onChange: ((Bool) -> Void)?

  override init() {
    super.init()
    synthesizer.delegate = self
  }

  /// Replies are read out automatically only through headphones; the
  /// speaker button reads them on the watch too.
  static var headphonesConnected: Bool {
    let headphones: Set<AVAudioSession.Port> = [.bluetoothA2DP, .bluetoothLE, .bluetoothHFP, .headphones, .airPlay]
    return AVAudioSession.sharedInstance().currentRoute.outputs.contains { headphones.contains($0.portType) }
  }

  func speak(_ text: String) {
    let spoken = SpokenText.prepare(text)
    guard !spoken.isEmpty else { return }
    stop()
    let session = AVAudioSession.sharedInstance()
    try? session.setCategory(.playback, mode: .voicePrompt, options: [.duckOthers])
    try? session.setActive(true)
    let utterance = AVSpeechUtterance(string: spoken)
    utterance.rate = AVSpeechUtteranceDefaultSpeechRate
    synthesizer.speak(utterance)
    setSpeaking(true)
  }

  func stop() {
    if synthesizer.isSpeaking {
      synthesizer.stopSpeaking(at: .immediate)
    }
    setSpeaking(false)
  }

  private func setSpeaking(_ value: Bool) {
    isSpeaking = value
    onChange?(value)
  }

  nonisolated func speechSynthesizer(
    _ synthesizer: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance
  ) {
    Task { @MainActor in
      self.setSpeaking(false)
      try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }
  }

  nonisolated func speechSynthesizer(
    _ synthesizer: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance
  ) {
    Task { @MainActor in self.setSpeaking(false) }
  }
}
