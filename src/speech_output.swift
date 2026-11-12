import AVFoundation
import Foundation

/// Hands Rust a sentence's audio: its id, mono Float32 samples at
/// `outputRate` (none in the last call), and whether that was all of it.
public typealias HexSpeechAudio = @convention(c) (UInt64, UnsafePointer<Float>?, Int, Bool) -> Void

private let outputRate: Double = 48_000
private let outputFormat = AVAudioFormat(
    commonFormat: .pcmFormatFloat32,
    sampleRate: outputRate,
    channels: 1,
    interleaved: false
)!
/// A sentence whose engine sends nothing for this long is given up on, so
/// the next one starts.
private let sentenceTimeout: TimeInterval = 20

/// Set once by `hex_speech_output_start`, before any sentence.
nonisolated(unsafe) private var deliver: HexSpeechAudio?
/// Created and used on the main thread only: AVSpeechSynthesizer calls back
/// on the main queue whatever thread asks it.
nonisolated(unsafe) private var reader: Reader?

private final class Sentence {
    let id: UInt64
    let utterance: AVSpeechUtterance
    var converter: AVAudioConverter?
    var finished = false

    init(id: UInt64, utterance: AVSpeechUtterance) {
        self.id = id
        self.utterance = utterance
    }
}

/// Writes sentences to audio one at a time: queued writes into one
/// synthesizer stall when the next sentence's voice is of another kind.
/// Used on the main thread only, hence unchecked.
private final class Reader: NSObject, AVSpeechSynthesizerDelegate, @unchecked Sendable {
    let synthesizer = AVSpeechSynthesizer()
    var queue: [Sentence] = []
    var current: Sentence?

    override init() {
        super.init()
        synthesizer.delegate = self
    }

    func add(_ sentence: Sentence) {
        queue.append(sentence)
        startNext()
    }

    func startNext() {
        guard current == nil, !queue.isEmpty else { return }
        let sentence = queue.removeFirst()
        current = sentence
        synthesizer.write(sentence.utterance) { [weak self] buffer in
            self?.received(buffer, for: sentence)
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + sentenceTimeout) { [weak self] in
            self?.finish(sentence)
        }
    }

    func received(_ buffer: AVAudioBuffer, for sentence: Sentence) {
        guard !sentence.finished, let pcm = buffer as? AVAudioPCMBuffer else { return }
        if pcm.frameLength == 0 {
            finish(sentence)
            return
        }
        convert(pcm, for: sentence)
    }

    /// Hands Rust `pcm` at the output's rate, or with `pcm` nil, what the
    /// converter still holds.
    func convert(_ pcm: AVAudioPCMBuffer?, for sentence: Sentence) {
        if let pcm, sentence.converter?.inputFormat != pcm.format {
            sentence.converter = AVAudioConverter(from: pcm.format, to: outputFormat)
            sentence.converter?.downmix = true
        }
        guard let converter = sentence.converter else { return }
        let ratio = outputRate / converter.inputFormat.sampleRate
        let frames = Double(pcm?.frameLength ?? 0) * ratio + 4096
        guard
            let output = AVAudioPCMBuffer(
                pcmFormat: outputFormat,
                frameCapacity: AVAudioFrameCount(frames)
            )
        else { return }
        var given = false
        var error: NSError?
        converter.convert(to: output, error: &error) { _, status in
            guard let pcm, !given else {
                status.pointee = pcm == nil ? .endOfStream : .noDataNow
                return nil
            }
            given = true
            status.pointee = .haveData
            return pcm
        }
        guard error == nil, output.frameLength > 0, let samples = output.floatChannelData?[0]
        else { return }
        deliver?(sentence.id, samples, Int(output.frameLength), false)
    }

    func finish(_ sentence: Sentence) {
        guard !sentence.finished else { return }
        convert(nil, for: sentence)
        sentence.finished = true
        deliver?(sentence.id, nil, 0, true)
        if current === sentence {
            current = nil
            startNext()
        }
    }

    func stop() {
        queue.removeAll()
        if let sentence = current {
            sentence.finished = true
            current = nil
        }
        synthesizer.stopSpeaking(at: .immediate)
    }

    func speechSynthesizer(_: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
        if let sentence = current, sentence.utterance === utterance {
            finish(sentence)
        }
    }

    func speechSynthesizer(_: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
        if let sentence = current, sentence.utterance === utterance {
            finish(sentence)
        }
    }
}

/// The main thread runs no run loop: a command-line `hex` calling from it.
private func mainLoopIdle() -> Bool {
    Thread.isMainThread && CFRunLoopCopyCurrentMode(CFRunLoopGetMain()) == nil
}

private func onMain(_ work: @escaping () -> Void) {
    if Thread.isMainThread {
        work()
    } else {
        DispatchQueue.main.async(execute: work)
    }
}

/// Registers where audio goes. Call once, before anything else.
@_cdecl("hex_speech_output_start")
public func hexSpeechOutputStart(_ callback: HexSpeechAudio) {
    deliver = callback
}

/// Whether the main queue answers, which the synthesizer needs: 1 when it
/// does, 2 on a main thread that runs no loop (sentences then pump it
/// while they are written), 0 when it didn't within `timeoutMs`.
@_cdecl("hex_speech_output_main_state")
public func hexSpeechOutputMainState(_ timeoutMs: Int32) -> Int32 {
    if Thread.isMainThread {
        return mainLoopIdle() ? 2 : 1
    }
    let answered = DispatchSemaphore(value: 0)
    DispatchQueue.main.async { answered.signal() }
    return answered.wait(timeout: .now() + .milliseconds(Int(timeoutMs))) == .success ? 1 : 0
}

/// Writes `text` to audio with the voice `voice` (an identifier; nil for
/// the system's) at `rate`, after the sentences before it. The audio goes
/// to the registered callback under `id`.
@_cdecl("hex_speech_output_speak")
public func hexSpeechOutputSpeak(
    _ id: UInt64,
    _ text: UnsafePointer<CChar>,
    _ voice: UnsafePointer<CChar>?,
    _ rate: Float
) {
    let utterance = AVSpeechUtterance(string: String(cString: text))
    if let voice, let chosen = AVSpeechSynthesisVoice(identifier: String(cString: voice)) {
        utterance.voice = chosen
    }
    utterance.rate = min(max(rate, AVSpeechUtteranceMinimumSpeechRate), AVSpeechUtteranceMaximumSpeechRate)
    utterance.preUtteranceDelay = 0
    utterance.postUtteranceDelay = 0
    let sentence = Sentence(id: id, utterance: utterance)
    onMain {
        if reader == nil {
            reader = Reader()
        }
        reader?.add(sentence)
    }
    if mainLoopIdle() {
        // Nothing else runs the main queue: run it until this sentence is
        // written (the synthesizer is far faster than reading it).
        let deadline = Date().addingTimeInterval(sentenceTimeout)
        while !sentence.finished, Date() < deadline {
            _ = CFRunLoopRunInMode(.defaultMode, 0.02, true)
        }
    }
}

/// Drops the queued sentences and stops writing the current one.
@_cdecl("hex_speech_output_stop")
public func hexSpeechOutputStop() {
    onMain { reader?.stop() }
}

private struct BridgeVoice: Codable {
    let id: String
    let name: String
    let language: String
    /// 1 default, 2 enhanced, 3 premium, 4 the person's Personal Voice.
    let quality: Int
}

/// The installed voices as JSON (owned; free it with
/// `hex_speech_output_free_string`). Personal Voices are listed only once
/// the person allowed Hex to use them.
@_cdecl("hex_speech_output_voices")
public func hexSpeechOutputVoices() -> UnsafeMutablePointer<CChar>? {
    let voices = AVSpeechSynthesisVoice.speechVoices().map { voice in
        let personal = voice.voiceTraits.contains(.isPersonalVoice)
        let quality: Int
        switch voice.quality {
        case .premium: quality = 3
        case .enhanced: quality = 2
        default: quality = 1
        }
        return BridgeVoice(
            id: voice.identifier,
            name: voice.name,
            language: voice.language,
            quality: personal ? 4 : quality
        )
    }
    guard let data = try? JSONEncoder().encode(voices) else { return nil }
    return strdup(String(decoding: data, as: UTF8.self))
}

@_cdecl("hex_speech_output_free_string")
public func hexSpeechOutputFreeString(_ string: UnsafeMutablePointer<CChar>?) {
    free(string)
}
