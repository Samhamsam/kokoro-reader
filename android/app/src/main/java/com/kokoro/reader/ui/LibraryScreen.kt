package com.kokoro.reader.ui

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.kokoro.reader.data.BookEntry
import com.kokoro.reader.data.Library

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LibraryScreen(
    library: Library,
    onOpenBook: (String) -> Unit,
    onSettings: () -> Unit,
    context: Context
) {
    var books by remember { mutableStateOf(library.books.toList()) }
    var searchQuery by remember { mutableStateOf("") }
    var deleteConfirm by remember { mutableStateOf<String?>(null) }

    val filePicker = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenDocument()
    ) { uri: Uri? ->
        uri?.let {
            val filename = getFileName(context, it) ?: "unknown.pdf"
            library.import(context, it, filename)
            books = library.books.toList()
        }
    }

    // Refresh on resume (off main thread)
    LaunchedEffect(Unit) {
        kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
            library.refresh()
        }
        books = library.books.toList()
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Text("Kokoro Reader", fontWeight = androidx.compose.ui.text.font.FontWeight.Bold)
                },
                actions = {
                    TextButton(onClick = onSettings) {
                        Text("Settings", color = TextDim)
                    }
                    Button(
                        onClick = { filePicker.launch(arrayOf("application/pdf")) },
                        colors = ButtonDefaults.buttonColors(containerColor = Accent)
                    ) {
                        Text("Import")
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Surface)
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(horizontal = 16.dp, vertical = 8.dp)
        ) {
            // Search
            OutlinedTextField(
                value = searchQuery,
                onValueChange = { searchQuery = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = { Text("Search books...") },
                singleLine = true
            )

            Spacer(Modifier.height(12.dp))

            val filtered = books
                .filter { searchQuery.isEmpty() || it.title.contains(searchQuery, ignoreCase = true) }
                .sortedByDescending { it.last_accessed }

            if (filtered.isEmpty()) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text("No books yet", fontSize = 22.sp, color = TextDim)
                        Spacer(Modifier.height(8.dp))
                        Text("Tap Import to add a PDF", fontSize = 14.sp, color = TextDim)
                    }
                }
            } else {
                LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(filtered, key = { it.id }) { book ->
                        BookCard(
                            book = book,
                            onClick = { onOpenBook(book.id) },
                            onDelete = { deleteConfirm = book.id }
                        )
                    }
                }
            }
        }
    }

    // Delete confirmation dialog
    deleteConfirm?.let { id ->
        AlertDialog(
            onDismissRequest = { deleteConfirm = null },
            title = { Text("Delete Book") },
            text = { Text("Are you sure you want to delete this book?") },
            confirmButton = {
                TextButton(onClick = {
                    library.delete(id)
                    books = library.books.toList()
                    deleteConfirm = null
                }) {
                    Text("Delete", color = Red)
                }
            },
            dismissButton = {
                TextButton(onClick = { deleteConfirm = null }) {
                    Text("Cancel")
                }
            }
        )
    }
}

@Composable
fun BookCard(book: BookEntry, onClick: () -> Unit, onDelete: () -> Unit) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        colors = CardDefaults.cardColors(containerColor = Surface),
        shape = RoundedCornerShape(8.dp)
    ) {
        Column(modifier = Modifier.padding(14.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    book.title,
                    style = MaterialTheme.typography.titleSmall,
                    color = TextPrimary,
                    modifier = Modifier.weight(1f)
                )
                TextButton(onClick = onDelete) {
                    Text("Delete", color = Red, fontSize = 12.sp)
                }
            }

            Spacer(Modifier.height(6.dp))

            // Progress bar
            LinearProgressIndicator(
                progress = { book.progress },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(6.dp)
                    .clip(RoundedCornerShape(3.dp)),
                color = Accent,
                trackColor = ProgressBg,
            )

            Spacer(Modifier.height(4.dp))

            Text(
                "${book.progressPercent}%  —  Page ${book.last_page + 1} / ${book.total_pages}",
                style = MaterialTheme.typography.bodySmall,
                color = TextDim
            )
        }
    }
}

private fun getFileName(context: Context, uri: Uri): String? {
    context.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
        val nameIndex = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
        cursor.moveToFirst()
        return cursor.getString(nameIndex)
    }
    return null
}
