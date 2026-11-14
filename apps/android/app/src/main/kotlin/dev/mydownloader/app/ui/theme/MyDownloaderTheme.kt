package dev.fetchx.app.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.unit.dp

private val FetchXDarkColorScheme = darkColorScheme(
    primary = Color(0xFF78A9FF),
    onPrimary = Color(0xFF001D6C),
    primaryContainer = Color(0xFF0043CE),
    onPrimaryContainer = Color(0xFFD0E2FF),
    secondary = Color(0xFFC6C6C6),
    onSecondary = Color(0xFF262626),
    secondaryContainer = Color(0xFF525252),
    onSecondaryContainer = Color(0xFFF4F4F4),
    tertiary = Color(0xFF42BE65),
    onTertiary = Color(0xFF071908),
    tertiaryContainer = Color(0xFF044317),
    onTertiaryContainer = Color(0xFFA7F0BA),
    background = Color(0xFF161616),
    onBackground = Color(0xFFF4F4F4),
    surface = Color(0xFF262626),
    onSurface = Color(0xFFF4F4F4),
    surfaceVariant = Color(0xFF393939),
    onSurfaceVariant = Color(0xFFC6C6C6),
    outline = Color(0xFF8D8D8D),
    outlineVariant = Color(0xFF525252),
    error = Color(0xFFFF8389),
    onError = Color(0xFF410002),
    errorContainer = Color(0xFFA2191F),
    onErrorContainer = Color(0xFFFFF1F1),
    surfaceContainerLowest = Color(0xFF161616),
    surfaceContainerLow = Color(0xFF262626),
    surfaceContainer = Color(0xFF393939),
    surfaceContainerHigh = Color(0xFF525252),
    surfaceContainerHighest = Color(0xFF6F6F6F),
)

private val FetchXLightColorScheme = lightColorScheme(
    primary = Color(0xFF0F62FE),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFD0E2FF),
    onPrimaryContainer = Color(0xFF001D6C),
    secondary = Color(0xFF393939),
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFE0E0E0),
    onSecondaryContainer = Color(0xFF161616),
    tertiary = Color(0xFF198038),
    onTertiary = Color.White,
    tertiaryContainer = Color(0xFFDEFBE6),
    onTertiaryContainer = Color(0xFF0E6027),
    background = Color(0xFFF4F4F4),
    onBackground = Color(0xFF161616),
    surface = Color.White,
    onSurface = Color(0xFF161616),
    surfaceVariant = Color(0xFFE8E8E8),
    onSurfaceVariant = Color(0xFF525252),
    outline = Color(0xFF6F6F6F),
    outlineVariant = Color(0xFFC6C6C6),
    error = Color(0xFFDA1E28),
    onError = Color.White,
    errorContainer = Color(0xFFFFF1F1),
    onErrorContainer = Color(0xFFA2191F),
    surfaceContainerLowest = Color.White,
    surfaceContainerLow = Color(0xFFF4F4F4),
    surfaceContainer = Color(0xFFE8E8E8),
    surfaceContainerHigh = Color(0xFFE0E0E0),
    surfaceContainerHighest = Color(0xFFC6C6C6),
)

private val FetchXShapes = Shapes(
    extraSmall = RoundedCornerShape(0.dp),
    small = RoundedCornerShape(0.dp),
    medium = RoundedCornerShape(0.dp),
    large = RoundedCornerShape(0.dp),
    extraLarge = RoundedCornerShape(0.dp),
)

@Composable
fun FetchXTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    seedColor: Color? = null,
    content: @Composable () -> Unit,
) {
    val colorScheme = when {
        seedColor != null -> {
            val onAccent = if (seedColor.luminance() > 0.179f) {
                Color(0xFF161616)
            } else {
                Color.White
            }
            if (darkTheme) {
                FetchXDarkColorScheme.copy(
                    primary = seedColor,
                    onPrimary = onAccent,
                    primaryContainer = lerp(Color(0xFF262626), seedColor, 0.36f),
                    onPrimaryContainer = Color(0xFFF4F4F4),
                )
            } else {
                FetchXLightColorScheme.copy(
                    primary = seedColor,
                    onPrimary = onAccent,
                    primaryContainer = lerp(Color.White, seedColor, 0.16f),
                    onPrimaryContainer = Color(0xFF161616),
                )
            }
        }
        darkTheme -> FetchXDarkColorScheme
        else -> FetchXLightColorScheme
    }

    MaterialTheme(
        colorScheme = colorScheme,
        shapes = FetchXShapes,
        content = content,
    )
}
