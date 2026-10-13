package dev.fetchx.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.fetchx.app.navigation.AppNavigation
import dev.fetchx.app.ui.theme.FetchXTheme

/**
 * Single-Activity entry point for FetchX Android.
 * Navigation is handled entirely by NavHost (Compose Navigation).
 */
class MainActivity : ComponentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            FetchXTheme {
                AppNavigation()
            }
        }
    }
}

