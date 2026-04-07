package com.kokoro.reader.ui

import android.os.Environment
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    currentDir: String,
    currentServerUrl: String,
    onSave: (dir: String, serverUrl: String) -> Unit
) {
    var path by remember { mutableStateOf(currentDir.ifEmpty {
        "${Environment.getExternalStorageDirectory()}/Syncthing/kokoro-reader"
    }) }
    var serverUrl by remember { mutableStateOf(currentServerUrl.ifEmpty {
        "http://192.168.1.100:8787"
    }) }

    val folderPicker = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenDocumentTree()
    ) { uri ->
        uri?.let {
            val docPath = it.path?.replace("/tree/primary:", "${Environment.getExternalStorageDirectory()}/")
            if (docPath != null) path = docPath
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = Surface)
            )
        }
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(20.dp)
        ) {
            // Data Folder
            Text("Data Folder", style = MaterialTheme.typography.titleMedium, color = TextPrimary)
            Spacer(Modifier.height(4.dp))
            Text(
                "Point this to your Syncthing folder to sync with Desktop.",
                style = MaterialTheme.typography.bodySmall, color = TextDim
            )
            Spacer(Modifier.height(12.dp))
            OutlinedTextField(
                value = path,
                onValueChange = { path = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                label = { Text("Path") }
            )
            Spacer(Modifier.height(8.dp))
            OutlinedButton(onClick = { folderPicker.launch(null) }) {
                Text("Browse")
            }

            Spacer(Modifier.height(24.dp))

            // TTS Server
            Text("TTS Server", style = MaterialTheme.typography.titleMedium, color = TextPrimary)
            Spacer(Modifier.height(4.dp))
            Text(
                "URL of the Go TTS server running on your homeserver or PC.",
                style = MaterialTheme.typography.bodySmall, color = TextDim
            )
            Spacer(Modifier.height(12.dp))
            OutlinedTextField(
                value = serverUrl,
                onValueChange = { serverUrl = it },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
                label = { Text("Server URL") },
                placeholder = { Text("http://192.168.1.100:8787") }
            )

            Spacer(Modifier.height(32.dp))

            Button(
                onClick = { onSave(path, serverUrl) },
                modifier = Modifier.fillMaxWidth(),
                colors = ButtonDefaults.buttonColors(containerColor = Accent)
            ) {
                Text("Save & Continue", fontSize = 16.sp)
            }
        }
    }
}
