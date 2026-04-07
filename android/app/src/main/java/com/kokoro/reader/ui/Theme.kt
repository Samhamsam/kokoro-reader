package com.kokoro.reader.ui

import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

val BgDark = Color(0xFF181820)
val Surface = Color(0xFF20212C)
val SurfaceHover = Color(0xFF2C2E3A)
val Accent = Color(0xFF6366F1)
val Green = Color(0xFF34D399)
val Amber = Color(0xFFFBBF24)
val Red = Color(0xFFEF4444)
val TextPrimary = Color(0xFFE2E8F0)
val TextDim = Color(0xFF788296)
val ProgressBg = Color(0xFF282A38)

private val DarkColorScheme = darkColorScheme(
    primary = Accent,
    secondary = Green,
    tertiary = Amber,
    background = BgDark,
    surface = Surface,
    onPrimary = Color.White,
    onSecondary = Color.White,
    onBackground = TextPrimary,
    onSurface = TextPrimary,
    error = Red,
)

@Composable
fun KokoroTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = DarkColorScheme,
        content = content
    )
}
