package dev.fetchx.app.ui.theme

import android.os.Build
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

private val DarkColorScheme = darkColorScheme()
private val LightColorScheme = lightColorScheme()

/**
 * FetchX Material 3 theme.
 *
 * [seedColor]: when non-null, overrides the wallpaper-based dynamic color with a
 * custom primary seed. On API 31+ we fall back to the wallpaper dynamic scheme when
 * seedColor is null. On older APIs we always use a static scheme derived from seedColor
 * (or the built-in default when seedColor is null).
 */
@Composable
fun FetchXTheme(
    darkTheme: Boolean = false,
    seedColor: Color? = null,
    content: @Composable () -> Unit,
) {
    val colorScheme = when {
        // Custom seed overrides dynamic wallpaper
        seedColor != null -> {
            if (darkTheme) {
                darkColorScheme(
                    primary = seedColor,
                    secondary = seedColor.copy(alpha = 0.7f),
                    tertiary = seedColor.copy(alpha = 0.5f),
                )
            } else {
                lightColorScheme(
                    primary = seedColor,
                    secondary = seedColor.copy(alpha = 0.7f),
                    tertiary = seedColor.copy(alpha = 0.5f),
                )
            }
        }
        // API 31+: wallpaper-based dynamic color
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        // Fallback static scheme
        darkTheme -> DarkColorScheme
        else -> LightColorScheme
    }

    MaterialTheme(
        colorScheme = colorScheme,
        content = content,
    )
}
