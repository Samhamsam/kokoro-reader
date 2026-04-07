package com.kokoro.reader

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.Settings
import androidx.activity.ComponentActivity
import com.tom_roush.pdfbox.android.PDFBoxResourceLoader
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.kokoro.reader.data.Library
import com.kokoro.reader.ui.LibraryScreen
import com.kokoro.reader.ui.ReaderScreen
import com.kokoro.reader.ui.SettingsScreen
import com.kokoro.reader.ui.KokoroTheme
import java.io.File

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        PDFBoxResourceLoader.init(applicationContext)
        requestStoragePermission()

        setContent {
            KokoroTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background
                ) {
                    val prefs = getSharedPreferences("kokoro", MODE_PRIVATE)
                    var dataDir by remember {
                        mutableStateOf(prefs.getString("data_dir", null))
                    }

                    val navController = rememberNavController()
                    val startDest = if (dataDir == null) "settings" else "library"

                    NavHost(navController, startDestination = startDest) {
                        composable("settings") {
                            SettingsScreen(
                                currentDir = dataDir ?: "",
                                onSave = { dir ->
                                    prefs.edit().putString("data_dir", dir).apply()
                                    dataDir = dir
                                    navController.navigate("library") {
                                        popUpTo("settings") { inclusive = true }
                                    }
                                }
                            )
                        }

                        composable("library") {
                            val dir = dataDir ?: return@composable
                            val library = remember(dir) { Library(File(dir)) }
                            LibraryScreen(
                                library = library,
                                onOpenBook = { bookId ->
                                    navController.navigate("reader/$bookId")
                                },
                                onSettings = {
                                    navController.navigate("settings")
                                },
                                context = this@MainActivity
                            )
                        }

                        composable("reader/{bookId}") { backStackEntry ->
                            val bookId = backStackEntry.arguments?.getString("bookId") ?: return@composable
                            val dir = dataDir ?: return@composable
                            val library = remember(dir) { Library(File(dir)) }
                            ReaderScreen(
                                library = library,
                                bookId = bookId,
                                onBack = { navController.popBackStack() }
                            )
                        }
                    }
                }
            }
        }
    }

    private fun requestStoragePermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            if (!Environment.isExternalStorageManager()) {
                val intent = Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION)
                intent.data = Uri.parse("package:$packageName")
                startActivity(intent)
            }
        } else {
            requestPermissions(
                arrayOf(
                    android.Manifest.permission.READ_EXTERNAL_STORAGE,
                    android.Manifest.permission.WRITE_EXTERNAL_STORAGE
                ),
                1
            )
        }
    }
}
