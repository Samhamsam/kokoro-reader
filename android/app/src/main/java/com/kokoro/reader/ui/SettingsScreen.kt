package com.kokoro.reader.ui

import android.content.Intent
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
fun SettingsScreen(currentDir: String, onSave: (String) -> Unit) {
    var path by remember { mutableStateOf(currentDir.ifEmpty {
        "${Environment.getExternalStorageDirectory()}/Syncthing/kokoro-reader"
    }) }

    val folderPicker = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenDocumentTree()
    ) { uri ->
        uri?.let {
            // Convert content URI to file path
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
            Text(
                "Data Folder",
                style = MaterialTheme.typography.titleMedium,
                color = TextPrimary
            )
            Spacer(Modifier.height(4.dp))
            Text(
                "Point this to your Syncthing folder to sync with Desktop.",
                style = MaterialTheme.typography.bodySmall,
                color = TextDim
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

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(onClick = { folderPicker.launch(null) }) {
                    Text("Browse")
                }
            }

            Spacer(Modifier.height(24.dp))

            Button(
                onClick = { onSave(path) },
                modifier = Modifier.fillMaxWidth(),
                colors = ButtonDefaults.buttonColors(containerColor = Accent)
            ) {
                Text("Save & Continue", fontSize = 16.sp)
            }
        }
    }
}
