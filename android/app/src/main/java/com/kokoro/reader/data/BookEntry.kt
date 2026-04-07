package com.kokoro.reader.data

data class BookEntry(
    val id: String = "",
    val title: String = "",
    val filename: String = "",
    val total_pages: Int = 0,
    val last_page: Int = 0,
    val last_sentence: Int = 0,
    val selected_voice: Int = 0,
    val last_accessed: Long = 0
) {
    val progress: Float
        get() = if (total_pages == 0) 0f else last_page.toFloat() / total_pages

    val progressPercent: Int
        get() = (progress * 100).toInt()
}

data class LibraryData(
    val books: List<BookEntry> = emptyList()
)
