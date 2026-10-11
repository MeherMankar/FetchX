package dev.mydownloader.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.mydownloader.app.navigation.AppNavigation
import dev.mydownloader.app.ui.theme.MyDownloaderTheme

/**
 * Single-Activity entry point for MyDownloader Android.
 * Navigation is handled entirely by NavHost (Compose Navigation).
 */
class MainActivity : ComponentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            MyDownloaderTheme {
                AppNavigation()
            }
        }
    }
}
