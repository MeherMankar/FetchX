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
    secondary = Color(0xFFC4D2D3),
    onSecondary = Color(0xFF172022),
    secondaryContainer = Color(0xFF344347),
    onSecondaryContainer = Color(0xFFE8F0F1),
    tertiary = Color(0xFF69C991),
    onTertiary = Color(0xFF102318),
    tertiaryContainer = Color(0xFF203A2B),
    onTertiaryContainer = Color(0xFFB5F1C9),
    background = Color(0xFF111517),
    onBackground = Color(0xFFEDF2F2),
    surface = Color(0xFF191F21),
    onSurface = Color(0xFFEDF2F2),
    surfaceVariant = Color(0xFF2D3638),
    onSurfaceVariant = Color(0xFFBCC8CA),
    outline = Color(0xFF89979A),
    outlineVariant = Color(0xFF414B4E),
    error = Color(0xFFFF9A9E),
    onError = Color(0xFF3B1518),
    errorContainer = Color(0xFF53282D),
    onErrorContainer = Color(0xFFFFDADD),
    surfaceContainerLowest = Color(0xFF111517),
    surfaceContainerLow = Color(0xFF191F21),
    surfaceContainer = Color(0xFF222A2C),
    surfaceContainerHigh = Color(0xFF2D3638),
    surfaceContainerHighest = Color(0xFF394447),
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
    extraSmall = RoundedCornerShape(4.dp),
    small = RoundedCornerShape(8.dp),
    medium = RoundedCornerShape(12.dp),
    large = RoundedCornerShape(16.dp),
    extraLarge = RoundedCornerShape(24.dp),
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
