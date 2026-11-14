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
    primary = Color(0xFF8ECAE6),
    onPrimary = Color(0xFF0D1417),
    primaryContainer = Color(0xFF1F3740),
    onPrimaryContainer = Color(0xFFEAF6FC),
    secondary = Color(0xFFC7D1D4),
    onSecondary = Color(0xFF101417),
    secondaryContainer = Color(0xFF1D2529),
    onSecondaryContainer = Color(0xFFEAF2F4),
    tertiary = Color(0xFF86D7A0),
    onTertiary = Color(0xFF0D1511),
    tertiaryContainer = Color(0xFF1E3528),
    onTertiaryContainer = Color(0xFFE1F7EA),
    background = Color(0xFF101417),
    onBackground = Color(0xFFEDF4F6),
    surface = Color(0xFF171D20),
    onSurface = Color(0xFFEDF4F6),
    surfaceVariant = Color(0xFF232D31),
    onSurfaceVariant = Color(0xFFC7D1D4),
    outline = Color(0xFF8C9AA0),
    outlineVariant = Color(0xFF2F3A3E),
    error = Color(0xFFF4A6A0),
    onError = Color(0xFF2A1717),
    errorContainer = Color(0xFF3C2827),
    onErrorContainer = Color(0xFFFFE4E2),
    surfaceContainerLowest = Color(0xFF101417),
    surfaceContainerLow = Color(0xFF171D20),
    surfaceContainer = Color(0xFF1D2529),
    surfaceContainerHigh = Color(0xFF232D31),
    surfaceContainerHighest = Color(0xFF2A3438),
)

private val FetchXLightColorScheme = lightColorScheme(
    primary = Color(0xFF2A6F8B),
    onPrimary = Color(0xFFEDF4F6),
    primaryContainer = Color(0xFFE8F3F8),
    onPrimaryContainer = Color(0xFF15313D),
    secondary = Color(0xFF3A4348),
    onSecondary = Color(0xFFEDF4F6),
    secondaryContainer = Color(0xFFE9EEF0),
    onSecondaryContainer = Color(0xFF1C2327),
    tertiary = Color(0xFF2D7A4E),
    onTertiary = Color(0xFFEDF4F6),
    tertiaryContainer = Color(0xFFE8F5ED),
    onTertiaryContainer = Color(0xFF183D2A),
    background = Color(0xFFF2F5F6),
    onBackground = Color(0xFF131A1D),
    surface = Color(0xFFFFFFFF),
    onSurface = Color(0xFF131A1D),
    surfaceVariant = Color(0xFFE7ECEE),
    onSurfaceVariant = Color(0xFF394549),
    outline = Color(0xFF617378),
    outlineVariant = Color(0xFFC2CDD1),
    error = Color(0xFFB94A4A),
    onError = Color(0xFFFFFFFF),
    errorContainer = Color(0xFFFDEEEE),
    onErrorContainer = Color(0xFF5A1D1D),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFF2F5F6),
    surfaceContainer = Color(0xFFE7ECEE),
    surfaceContainerHigh = Color(0xFFDBE1E4),
    surfaceContainerHighest = Color(0xFFC8D5D9),
)

private val FetchXShapes = Shapes(
    extraSmall = RoundedCornerShape(4.dp),
    small = RoundedCornerShape(8.dp),
    medium = RoundedCornerShape(10.dp),
    large = RoundedCornerShape(14.dp),
    extraLarge = RoundedCornerShape(20.dp),
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
