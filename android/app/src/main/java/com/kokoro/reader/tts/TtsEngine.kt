package com.kokoro.reader.tts

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import kotlinx.coroutines.*
import java.util.Locale
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

enum class TtsState {
    IDLE, GENERATING, PLAYING, PAUSED, FINISHED, ERROR
}

class TtsEngine(context: Context) {
    private var tts: TextToSpeech? = null
    private var ttsReady = false

    var state: TtsState = TtsState.IDLE
        private set
    var currentSentence: Int = 0
        private set
    var totalSentences: Int = 0
        private set
    var errorMessage: String? = null
        private set

    private var sentences: List<String> = emptyList()
    private val stopped = AtomicBoolean(false)
    private val paused = AtomicBoolean(false)
    private var speakJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.Main + SupervisorJob())

    init {
        tts = TextToSpeech(context) { status ->
            ttsReady = status == TextToSpeech.SUCCESS
            if (ttsReady) {
                tts?.language = Locale.US
            }
        }
    }

    fun speak(text: String, onStateChange: () -> Unit) {
        stop()
        val splitSentences = splitIntoSentences(text)
        if (splitSentences.isEmpty()) {
            state = TtsState.FINISHED
            onStateChange()
            return
        }

        sentences = splitSentences
        totalSentences = sentences.size
        currentSentence = 0
        stopped.set(false)
        paused.set(false)
        state = TtsState.PLAYING

        speakJob = scope.launch {
            for ((i, sentence) in sentences.withIndex()) {
                if (stopped.get()) break

                while (paused.get() && !stopped.get()) {
                    delay(100)
                }
                if (stopped.get()) break

                currentSentence = i
                onStateChange()

                // Speak and wait for completion
                val done = CompletableDeferred<Unit>()
                tts?.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
                    override fun onStart(utteranceId: String?) {}
                    override fun onDone(utteranceId: String?) { done.complete(Unit) }
                    override fun onError(utteranceId: String?) { done.complete(Unit) }
                })
                tts?.speak(sentence, TextToSpeech.QUEUE_FLUSH, null, "s$i")
                done.await()
            }

            if (!stopped.get()) {
                state = TtsState.FINISHED
                onStateChange()
            }
        }
        onStateChange()
    }

    fun speakFromSentence(text: String, startSentence: Int, onStateChange: () -> Unit) {
        stop()
        val splitSentences = splitIntoSentences(text)
        if (splitSentences.isEmpty() || startSentence >= splitSentences.size) {
            state = TtsState.FINISHED
            onStateChange()
            return
        }

        sentences = splitSentences
        totalSentences = sentences.size
        currentSentence = startSentence
        stopped.set(false)
        paused.set(false)
        state = TtsState.PLAYING

        speakJob = scope.launch {
            for (i in startSentence until sentences.size) {
                if (stopped.get()) break

                while (paused.get() && !stopped.get()) {
                    delay(100)
                }
                if (stopped.get()) break

                currentSentence = i
                onStateChange()

                val done = CompletableDeferred<Unit>()
                tts?.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
                    override fun onStart(utteranceId: String?) {}
                    override fun onDone(utteranceId: String?) { done.complete(Unit) }
                    override fun onError(utteranceId: String?) { done.complete(Unit) }
                })
                tts?.speak(sentences[i], TextToSpeech.QUEUE_FLUSH, null, "s$i")
                done.await()
            }

            if (!stopped.get()) {
                state = TtsState.FINISHED
                onStateChange()
            }
        }
        onStateChange()
    }

    fun pause() {
        paused.set(true)
        tts?.stop()
        state = TtsState.PAUSED
    }

    fun resume(onStateChange: () -> Unit) {
        paused.set(false)
        state = TtsState.PLAYING
        // Re-speak from current sentence
        val text = sentences.drop(currentSentence).joinToString(" ")
        // Actually, just continue the coroutine — it's waiting in the while(paused) loop
        onStateChange()
    }

    fun stop() {
        stopped.set(true)
        paused.set(false)
        tts?.stop()
        speakJob?.cancel()
        state = TtsState.IDLE
        currentSentence = 0
    }

    fun setSpeed(speed: Float) {
        tts?.setSpeechRate(speed)
    }

    fun setLanguage(locale: Locale) {
        tts?.language = locale
    }

    fun getCurrentSentenceText(): String? = sentences.getOrNull(currentSentence)

    fun release() {
        stop()
        tts?.shutdown()
        scope.cancel()
    }

    companion object {
        private fun prepareForTts(text: String): String {
            // Remove page numbers (standalone numbers on their own line)
            val filtered = text.lines().filter { line ->
                val trimmed = line.trim()
                if (trimmed.toIntOrNull() != null) return@filter false
                val stripped = trimmed.replace("-", "").replace("—", "").trim()
                if (stripped.isNotEmpty() && stripped.toIntOrNull() != null) return@filter false
                true
            }.joinToString(" ")
            return filtered
                .replace(" (", ", (").replace(" [", ", [")
                .replace(") ", "), ").replace("] ", "], ")
                .replace(" — ", ", — ").replace(" – ", ", – ").replace(" - ", ", — ")
                .replace(",,", ",").replace(", ,", ",")
        }

        /** Apply TTS preprocessing to a single sentence before sending to server */
        fun prepareForServer(text: String): String = prepareForTts(text)

        fun splitIntoSentences(text: String): List<String> {
            // NOTE: prepareForTts NOT applied here — keeps original text for display.
            // Applied per-sentence in ServerTtsEngine before sending to server.
            val cleaned = text
                .replace(Regex("[\\p{Cntrl}]"), " ")
                .replace(Regex("[♦♣♠♥★☆●○◆◇■□▪▫▲△▼▽•‣⁃※†‡§¶]"), " ")
                .replace(Regex("\\s+"), " ")
                .trim()

            if (cleaned.isEmpty()) return emptyList()

            val sentences = mutableListOf<String>()
            val current = StringBuilder()

            for (ch in cleaned) {
                current.append(ch)
                if (ch in ".!?;") {
                    // Check for abbreviations (single uppercase letter before dot)
                    val str = current.toString().trim()
                    if (ch == '.' && isAbbreviation(str)) continue

                    if (str.isNotEmpty()) sentences.add(str)
                    current.clear()
                }
                if (current.length > 400) {
                    val str = current.toString()
                    val lastSpace = str.lastIndexOf(' ')
                    if (lastSpace > 0) {
                        sentences.add(str.substring(0, lastSpace).trim())
                        current.clear()
                        current.append(str.substring(lastSpace).trim())
                    }
                }
            }
            val remainder = current.toString().trim()
            if (remainder.isNotEmpty()) sentences.add(remainder)

            return sentences.filter { it.count { c -> c.isLetter() } >= 2 }
        }

        private val abbreviations = setOf(
            "Mr.", "Mrs.", "Ms.", "Dr.", "Prof.", "Sr.", "Jr.", "St.", "Mt.",
            "Rev.", "Gen.", "Gov.", "vs.", "etc.", "approx.", "dept.",
            "i.e.", "e.g.", "U.S.", "U.K.", "a.m.", "p.m."
        )

        private fun isAbbreviation(text: String): Boolean {
            if (!text.endsWith('.')) return false
            // Single uppercase letter: "D.", "J."
            if (text.length >= 2) {
                val beforeDot = text[text.length - 2]
                if (beforeDot.isUpperCase() && (text.length == 2 || text[text.length - 3] == ' ' || text[text.length - 3] == '.')) {
                    return true
                }
            }
            return abbreviations.any { text.endsWith(it) }
        }
    }
}
