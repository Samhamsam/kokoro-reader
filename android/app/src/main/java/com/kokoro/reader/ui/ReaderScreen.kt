package com.kokoro.reader.ui

import android.graphics.Bitmap
import android.graphics.pdf.PdfRenderer
import android.os.ParcelFileDescriptor
import com.tom_roush.pdfbox.pdmodel.PDDocument
import com.tom_roush.pdfbox.text.PDFTextStripper
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.kokoro.reader.data.Library
import com.kokoro.reader.tts.ServerTtsEngine
import com.kokoro.reader.tts.ServerVoice
import com.kokoro.reader.tts.TtsState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReaderScreen(
    library: Library,
    bookId: String,
    serverUrl: String,
    onBack: () -> Unit
) {
    val book = library.books.find { it.id == bookId } ?: run { onBack(); return }
    var pdfFile by remember { mutableStateOf<File?>(null) }

    var currentPage by remember { mutableIntStateOf(book.last_page) }
    var totalPages by remember { mutableIntStateOf(book.total_pages) }
    var bitmap by remember { mutableStateOf<Bitmap?>(null) }
    var pageText by remember { mutableStateOf("") }
    var loading by remember { mutableStateOf(true) }
    var speed by remember { mutableFloatStateOf(1.0f) }
    var voiceMenuExpanded by remember { mutableStateOf(false) }

    // TTS via server
    val ttsEngine = remember { ServerTtsEngine(serverUrl) }
    var voices by remember { mutableStateOf<List<ServerVoice>>(emptyList()) }
    var selectedVoice by remember { mutableStateOf(book.selected_voice_id.ifEmpty { "kokoro_heart" }) }
    var ttsState by remember { mutableStateOf(TtsState.IDLE) }
    var currentSentenceIdx by remember { mutableIntStateOf(0) }
    var readingActive by remember { mutableStateOf(false) }
    var serverConnected by remember { mutableStateOf(false) }

    val noopCallback: () -> Unit = {} // TTS engine callback does nothing — we poll instead

    // Fetch voices from server on start
    LaunchedEffect(serverUrl) {
        ttsEngine.updateServerUrl(serverUrl)
        val fetched = ttsEngine.fetchVoices()
        voices = fetched
        serverConnected = fetched.isNotEmpty()
        // Validate saved voice ID — if not found on server, use first available
        if (fetched.isNotEmpty() && fetched.none { it.id == selectedVoice }) {
            selectedVoice = fetched.first().id
        }
    }

    // Poll TTS state every 200ms — simple and no recomposition loops
    LaunchedEffect(readingActive) {
        if (!readingActive) return@LaunchedEffect

        // Wait for TTS to actually start before polling
        // (avoid treating initial IDLE/FINISHED as "done")
        var waitingForStart = true

        while (readingActive) {
            val newState = ttsEngine.state
            val newSentence = ttsEngine.currentSentence

            // Once we see PLAYING or GENERATING, we know TTS has started
            if (waitingForStart) {
                if (newState == TtsState.PLAYING || newState == TtsState.GENERATING) {
                    waitingForStart = false
                }
                // Don't process FINISHED while waiting for start
                ttsState = newState
                currentSentenceIdx = newSentence
                kotlinx.coroutines.delay(100)
                continue
            }

            if (newState != ttsState) ttsState = newState
            if (newSentence != currentSentenceIdx) currentSentenceIdx = newSentence

            // Auto-advance when finished
            if (newState == TtsState.FINISHED) {
                if (currentPage + 1 < totalPages) {
                    currentPage++
                    kotlinx.coroutines.delay(300)
                    loading = true
                    val file = pdfFile ?: continue
                    val result = withContext(Dispatchers.IO) { renderPage(file, currentPage) }
                    bitmap = result?.first
                    pageText = result?.second ?: ""
                    loading = false
                    library.updateProgress(bookId, currentPage, 0, selectedVoice)
                    if (pageText.isNotBlank()) {
                        waitingForStart = true // reset for next page
                        ttsEngine.speak(pageText, selectedVoice, speed, onStateChange = noopCallback)
                    }
                } else {
                    readingActive = false
                }
            }

            kotlinx.coroutines.delay(200)
        }
        ttsState = ttsEngine.state
    }

    // Download PDF if needed, then render page
    LaunchedEffect(currentPage) {
        if (!readingActive) {
            loading = true
            // Download PDF on IO thread if not cached
            if (pdfFile == null || !pdfFile!!.exists()) {
                pdfFile = withContext(Dispatchers.IO) { library.getBookFile(book.id) }
            }
            val file = pdfFile ?: run { loading = false; return@LaunchedEffect }
            val result = withContext(Dispatchers.IO) { renderPage(file, currentPage) }
            bitmap = result?.first
            pageText = result?.second ?: ""
            loading = false

            if (totalPages == 0) {
                totalPages = withContext(Dispatchers.IO) { getPageCount(file) }
            }
            withContext(Dispatchers.IO) {
                library.updateProgress(bookId, currentPage, 0, selectedVoice)
            }
        }
    }

    DisposableEffect(Unit) {
        onDispose {
            ttsEngine.stop()
            library.updateProgress(bookId, currentPage, ttsEngine.currentSentence, selectedVoice)
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        TextButton(onClick = {
                            ttsEngine.stop()
                            library.updateProgress(bookId, currentPage, ttsEngine.currentSentence, selectedVoice)
                            onBack()
                        }) { Text("< Library", color = TextPrimary) }

                        Spacer(Modifier.width(8.dp))

                        IconButton(
                            onClick = { ttsEngine.stop(); readingActive = false; if (currentPage > 0) currentPage-- },
                            enabled = currentPage > 0
                        ) { Text("<", color = TextPrimary, fontSize = 18.sp) }

                        Text("${currentPage + 1}/$totalPages", color = TextDim, fontSize = 14.sp)

                        IconButton(
                            onClick = { ttsEngine.stop(); readingActive = false; if (currentPage + 1 < totalPages) currentPage++ },
                            enabled = currentPage + 1 < totalPages
                        ) { Text(">", color = TextPrimary, fontSize = 18.sp) }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Surface)
            )
        },
        bottomBar = {
            Surface(color = Surface, tonalElevation = 4.dp) {
                Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
                    if (!serverConnected) {
                        Text("Server not reachable: $serverUrl", color = Red, fontSize = 12.sp)
                        Spacer(Modifier.height(4.dp))
                    }

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        when (ttsState) {
                            TtsState.IDLE, TtsState.FINISHED, TtsState.ERROR -> {
                                Button(
                                    onClick = {
                                        ttsEngine.speak(pageText, selectedVoice, speed, onStateChange = noopCallback)
                                        readingActive = true // set AFTER speak so polling doesn't see FINISHED
                                    },
                                    enabled = pageText.isNotBlank() && serverConnected,
                                    colors = ButtonDefaults.buttonColors(containerColor = Green)
                                ) { Text("Play") }
                            }
                            TtsState.GENERATING -> {
                                CircularProgressIndicator(modifier = Modifier.size(24.dp), color = Amber, strokeWidth = 2.dp)
                                Text("Generating...", color = Amber, fontSize = 12.sp)
                            }
                            TtsState.PLAYING -> {
                                Button(
                                    onClick = { ttsEngine.pause(); ttsState = ttsEngine.state },
                                    colors = ButtonDefaults.buttonColors(containerColor = Amber)
                                ) { Text("Pause") }
                                Button(
                                    onClick = { ttsEngine.stop(); readingActive = false; ttsState = ttsEngine.state },
                                    colors = ButtonDefaults.buttonColors(containerColor = Red)
                                ) { Text("Stop") }
                            }
                            TtsState.PAUSED -> {
                                Button(
                                    onClick = { ttsEngine.resume(noopCallback) },
                                    colors = ButtonDefaults.buttonColors(containerColor = Green)
                                ) { Text("Resume") }
                                Button(
                                    onClick = { ttsEngine.stop(); readingActive = false; ttsState = ttsEngine.state },
                                    colors = ButtonDefaults.buttonColors(containerColor = Red)
                                ) { Text("Stop") }
                            }
                        }

                        Spacer(Modifier.weight(1f))

                        // Voice selector
                        Box {
                            TextButton(onClick = { voiceMenuExpanded = true }) {
                                val voiceName = voices.find { it.id == selectedVoice }?.name ?: selectedVoice
                                Text(voiceName, color = TextPrimary, fontSize = 12.sp)
                            }
                            DropdownMenu(expanded = voiceMenuExpanded, onDismissRequest = { voiceMenuExpanded = false }) {
                                voices.forEach { voice ->
                                    DropdownMenuItem(
                                        text = { Text(voice.name) },
                                        onClick = { selectedVoice = voice.id; voiceMenuExpanded = false }
                                    )
                                }
                            }
                        }
                    }

                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Text("Speed: ${String.format("%.1f", speed)}x", color = TextDim, fontSize = 12.sp)
                        Slider(value = speed, onValueChange = { speed = it }, valueRange = 0.5f..2.0f, steps = 5, modifier = Modifier.weight(1f))
                    }
                }
            }
        }
    ) { padding ->
        Box(modifier = Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.TopCenter) {
            if (loading) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(color = Accent)
                }
            } else {
                Column(modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                    bitmap?.let { bmp ->
                        Image(bitmap = bmp.asImageBitmap(), contentDescription = "Page", modifier = Modifier.fillMaxWidth())
                    }
                    if (ttsState == TtsState.PLAYING || ttsState == TtsState.PAUSED) {
                        ttsEngine.getCurrentSentenceText()?.let { sentence ->
                            Card(
                                modifier = Modifier.fillMaxWidth().padding(8.dp),
                                colors = CardDefaults.cardColors(containerColor = Accent.copy(alpha = 0.15f))
                            ) {
                                Text(sentence, modifier = Modifier.padding(12.dp), color = TextPrimary, fontSize = 14.sp)
                            }
                        }
                    }
                }
            }
        }
    }
}

private fun renderPage(file: File, pageIndex: Int): Pair<Bitmap, String>? {
    return try {
        val fd = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        val renderer = PdfRenderer(fd)
        val idx = pageIndex.coerceIn(0, renderer.pageCount - 1)
        val page = renderer.openPage(idx)

        val scale = 2
        val bitmap = Bitmap.createBitmap(page.width * scale, page.height * scale, Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(android.graphics.Color.WHITE)
        page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
        page.close()
        renderer.close()
        fd.close()

        val text = try {
            val doc = PDDocument.load(file)
            val stripper = PDFTextStripper()
            stripper.startPage = idx + 1
            stripper.endPage = idx + 1
            val t = stripper.getText(doc)
            doc.close()
            t.trim()
        } catch (e: Exception) { "" }

        Pair(bitmap, text)
    } catch (e: Exception) { null }
}

private fun getPageCount(file: File): Int {
    return try {
        val fd = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        val renderer = PdfRenderer(fd)
        val count = renderer.pageCount
        renderer.close()
        fd.close()
        count
    } catch (e: Exception) { 0 }
}
