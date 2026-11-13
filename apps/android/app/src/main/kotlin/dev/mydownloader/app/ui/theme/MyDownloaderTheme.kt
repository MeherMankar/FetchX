package dev.fetchx.app.ui.theme

import android.os.Build
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.LocalContext
import androidx.compose.foundation.isSystemInDarkTheme

private val FetchXDarkColorScheme = darkColorScheme(
    primary = Color(0xFF9CCBFF),
    onPrimary = Color(0xFF003258),
    primaryContainer = Color(0xFF164A75),
    onPrimaryContainer = Color(0xFFD1E8FF),
    secondary = Color(0xFFB8C8DA),
    onSecondary = Color(0xFF22313F),
    secondaryContainer = Color(0xFF394957),
    onSecondaryContainer = Color(0xFFD4E5F5),
    tertiary = Color(0xFF8FD0B0),
    onTertiary = Color(0xFF003822),
    tertiaryContainer = Color(0xFF005333),
    onTertiaryContainer = Color(0xFFAAEFC7),
    background = Color(0xFF101418),
    onBackground = Color(0xFFE1E2E5),
    surface = Color(0xFF101418),
    onSurface = Color(0xFFE1E2E5),
    surfaceVariant = Color(0xFF41474E),
    onSurfaceVariant = Color(0xFFC1C7CF),
    outline = Color(0xFF8B9198),
    outlineVariant = Color(0xFF41474E),
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
    errorContainer = Color(0xFF93000A),
    onErrorContainer = Color(0xFFFFDAD6),
)

private val FetchXLightColorScheme = lightColorScheme(
    primary = Color(0xFF00639A),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFCDE5FF),
    onPrimaryContainer = Color(0xFF001D33),
    secondary = Color(0xFF50606F),
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFD4E5F5),
    onSecondaryContainer = Color(0xFF0D1D2A),
    tertiary = Color(0xFF006D42),
    onTertiary = Color.White,
    tertiaryContainer = Color(0xFF9AF6C2),
    onTertiaryContainer = Color(0xFF002110),
    background = Color(0xFFF9F9FC),
    onBackground = Color(0xFF191C20),
    surface = Color(0xFFF9F9FC),
    onSurface = Color(0xFF191C20),
    surfaceVariant = Color(0xFFDDE3EA),
    onSurfaceVariant = Color(0xFF41474E),
    outline = Color(0xFF72787F),
    outlineVariant = Color(0xFFC1C7CF),
    error = Color(0xFFBA1A1A),
    onError = Color.White,
    errorContainer = Color(0xFFFFDAD6),
    onErrorContainer = Color(0xFF410002),
)

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
    darkTheme: Boolean = isSystemInDarkTheme(),
    seedColor: Color? = null,
    content: @Composable () -> Unit,
) {
    val colorScheme = when {
        // Keep FetchX surfaces and semantic colors stable while the accent follows the saved seed.
        seedColor != null -> {
            val onAccent = if (seedColor.luminance() > 0.179f) {
                Color(0xFF111827)
            } else {
                Color.White
            }
            if (darkTheme) {
                FetchXDarkColorScheme.copy(
                    primary = seedColor,
                    onPrimary = onAccent,
                    primaryContainer = lerp(Color(0xFF19191B), seedColor, 0.36f),
                    onPrimaryContainer = Color(0xFFEDF2F8),
                )
            } else {
                FetchXLightColorScheme.copy(
                    primary = seedColor,
                    onPrimary = onAccent,
                    primaryContainer = lerp(Color.White, seedColor, 0.16f),
                    onPrimaryContainer = Color(0xFF171717),
                )
            }
        }
        // API 31+: wallpaper-based dynamic color
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        }
        // Fallback static scheme
        darkTheme -> FetchXDarkColorScheme
        else -> FetchXLightColorScheme
    }

    MaterialTheme(
        colorScheme = colorScheme,
        content = content,
    )
}
