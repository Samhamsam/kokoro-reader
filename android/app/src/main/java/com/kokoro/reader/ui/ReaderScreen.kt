package com.kokoro.reader.ui

import android.graphics.Bitmap
import android.graphics.pdf.PdfRenderer
import android.os.ParcelFileDescriptor
import androidx.compose.foundation.Image
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.kokoro.reader.data.Library
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReaderScreen(
    library: Library,
    bookId: String,
    onBack: () -> Unit
) {
    val book = library.books.find { it.id == bookId } ?: run {
        onBack()
        return
    }
    val pdfFile = library.getBookPath(book)

    var currentPage by remember { mutableIntStateOf(book.last_page) }
    var totalPages by remember { mutableIntStateOf(book.total_pages) }
    var bitmap by remember { mutableStateOf<Bitmap?>(null) }
    var loading by remember { mutableStateOf(true) }

    // Render page
    LaunchedEffect(currentPage) {
        loading = true
        bitmap = withContext(Dispatchers.IO) {
            renderPage(pdfFile, currentPage)
        }
        loading = false

        // Update progress
        library.updateProgress(bookId, currentPage, 0, book.selected_voice)

        // Update total pages on first load
        if (totalPages == 0) {
            val count = withContext(Dispatchers.IO) { getPageCount(pdfFile) }
            totalPages = count
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        TextButton(onClick = onBack) {
                            Text("< Library", color = TextPrimary)
                        }
                        Spacer(Modifier.width(12.dp))
                        // Page navigation
                        IconButton(
                            onClick = { if (currentPage > 0) currentPage-- },
                            enabled = currentPage > 0
                        ) {
                            Text("<", color = TextPrimary, fontSize = 18.sp)
                        }
                        Text(
                            "${currentPage + 1} / $totalPages",
                            color = TextDim,
                            fontSize = 14.sp
                        )
                        IconButton(
                            onClick = { if (currentPage + 1 < totalPages) currentPage++ },
                            enabled = currentPage + 1 < totalPages
                        ) {
                            Text(">", color = TextPrimary, fontSize = 18.sp)
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Surface)
            )
        }
    ) { padding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
            contentAlignment = Alignment.Center
        ) {
            if (loading) {
                CircularProgressIndicator(color = Accent)
            } else {
                bitmap?.let { bmp ->
                    Image(
                        bitmap = bmp.asImageBitmap(),
                        contentDescription = "Page ${currentPage + 1}",
                        modifier = Modifier.fillMaxWidth()
                    )
                }
            }
        }
    }
}

private fun renderPage(file: File, pageIndex: Int): Bitmap? {
    return try {
        val fd = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        val renderer = PdfRenderer(fd)
        val page = renderer.openPage(pageIndex.coerceIn(0, renderer.pageCount - 1))

        val scale = 2 // 2x for crisp rendering
        val bitmap = Bitmap.createBitmap(
            page.width * scale, page.height * scale, Bitmap.Config.ARGB_8888
        )
        bitmap.eraseColor(android.graphics.Color.WHITE)
        page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
        page.close()
        renderer.close()
        fd.close()
        bitmap
    } catch (e: Exception) {
        e.printStackTrace()
        null
    }
}

private fun getPageCount(file: File): Int {
    return try {
        val fd = ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
        val renderer = PdfRenderer(fd)
        val count = renderer.pageCount
        renderer.close()
        fd.close()
        count
    } catch (e: Exception) {
        0
    }
}
