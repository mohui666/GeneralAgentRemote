package dev.agentremote.messenger.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

private val LightColors = lightColorScheme(
    primary = Color(0xFF33363B),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFEEEDE9),
    onPrimaryContainer = Color(0xFF282A2D),
    secondary = Color(0xFF6B6D70),
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFEEEDE9),
    onSecondaryContainer = Color(0xFF282A2D),
    tertiary = Color(0xFF6B6D70),
    onTertiary = Color.White,
    tertiaryContainer = Color(0xFFF1F0ED),
    onTertiaryContainer = Color(0xFF282A2D),
    background = Color(0xFFFBFAF8),
    onBackground = Color(0xFF282A2D),
    surface = Color.White,
    onSurface = Color(0xFF282A2D),
    surfaceVariant = Color(0xFFF7F6F3),
    onSurfaceVariant = Color(0xFF6B6D70),
    surfaceTint = Color(0xFF282A2D),
    surfaceBright = Color.White,
    surfaceDim = Color(0xFFEEEDE9),
    surfaceContainerLowest = Color.White,
    surfaceContainerLow = Color(0xFFFBFAF8),
    surfaceContainer = Color(0xFFF7F6F3),
    surfaceContainerHigh = Color(0xFFF1F0ED),
    surfaceContainerHighest = Color(0xFFEEEDE9),
    outline = Color(0xFFE2E0DA),
    outlineVariant = Color(0xFFE2E0DA),
    inverseSurface = Color(0xFF33363B),
    inverseOnSurface = Color(0xFFF7F6F3),
    inversePrimary = Color(0xFFE1E0DB),
    error = Color(0xFF9C3938),
    onError = Color.White,
    errorContainer = Color(0xFFFBEFED),
    onErrorContainer = Color(0xFF9C3938),
    scrim = Color(0xFF202226),
)

private val DarkColors = darkColorScheme(
    primary = Color(0xFFE1E0DB),
    onPrimary = Color(0xFF242528),
    primaryContainer = Color(0xFF2D2E31),
    onPrimaryContainer = Color(0xFFE4E3DF),
    secondary = Color(0xFFA5A5A3),
    onSecondary = Color(0xFF242528),
    secondaryContainer = Color(0xFF2D2E31),
    onSecondaryContainer = Color(0xFFE4E3DF),
    tertiary = Color(0xFFA5A5A3),
    onTertiary = Color(0xFF242528),
    tertiaryContainer = Color(0xFF27282B),
    onTertiaryContainer = Color(0xFFE4E3DF),
    background = Color(0xFF1C1D1F),
    onBackground = Color(0xFFE4E3DF),
    surface = Color(0xFF232426),
    onSurface = Color(0xFFE4E3DF),
    surfaceVariant = Color(0xFF27282B),
    onSurfaceVariant = Color(0xFFA5A5A3),
    surfaceTint = Color(0xFFE4E3DF),
    surfaceBright = Color(0xFF35363A),
    surfaceDim = Color(0xFF18191B),
    surfaceContainerLowest = Color(0xFF18191B),
    surfaceContainerLow = Color(0xFF1C1D1F),
    surfaceContainer = Color(0xFF232426),
    surfaceContainerHigh = Color(0xFF27282B),
    surfaceContainerHighest = Color(0xFF2D2E31),
    outline = Color(0xFF343538),
    outlineVariant = Color(0xFF343538),
    inverseSurface = Color(0xFFE1E0DB),
    inverseOnSurface = Color(0xFF242528),
    inversePrimary = Color(0xFF33363B),
    error = Color(0xFFE7A6A1),
    onError = Color(0xFF352827),
    errorContainer = Color(0xFF352827),
    onErrorContainer = Color(0xFFE7A6A1),
    scrim = Color(0xFF090A0B),
)

private data class RemoteSemanticColors(val accent: Color, val success: Color, val warning: Color)

private val LightSemanticColors = RemoteSemanticColors(
    accent = Color(0xFF315FB4),
    success = Color(0xFF6B6D70),
    warning = Color(0xFF8B632D),
)
private val DarkSemanticColors = RemoteSemanticColors(
    accent = Color(0xFF95B4EC),
    success = Color(0xFFA5A5A3),
    warning = Color(0xFFCBB183),
)
private val LocalRemoteSemanticColors = staticCompositionLocalOf { LightSemanticColors }

internal val RemoteBackground: Color @Composable get() = MaterialTheme.colorScheme.background
internal val RemoteSurface: Color @Composable get() = MaterialTheme.colorScheme.surface
internal val RemoteSurfaceRaised: Color @Composable get() = MaterialTheme.colorScheme.surfaceVariant
internal val RemoteBorder: Color @Composable get() = MaterialTheme.colorScheme.outline
internal val RemoteText: Color @Composable get() = MaterialTheme.colorScheme.onSurface
internal val RemoteMuted: Color @Composable get() = MaterialTheme.colorScheme.onSurfaceVariant
internal val RemoteAccent: Color @Composable get() = LocalRemoteSemanticColors.current.accent
internal val RemoteSuccess: Color @Composable get() = LocalRemoteSemanticColors.current.success
internal val RemoteWarning: Color @Composable get() = LocalRemoteSemanticColors.current.warning

private val AgentRemoteTypography = Typography(
    displayLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 32.sp, lineHeight = 40.sp, fontWeight = FontWeight.Medium),
    displayMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 28.sp, lineHeight = 36.sp, fontWeight = FontWeight.Medium),
    displaySmall = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 26.sp, lineHeight = 34.sp, fontWeight = FontWeight.Medium),
    headlineLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 26.sp, lineHeight = 34.sp, fontWeight = FontWeight.Medium),
    headlineMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 24.sp, lineHeight = 32.sp, fontWeight = FontWeight.Medium),
    headlineSmall = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 22.sp, lineHeight = 30.sp, fontWeight = FontWeight.Medium),
    titleLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 20.sp, lineHeight = 28.sp, fontWeight = FontWeight.Medium),
    titleMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 16.sp, lineHeight = 24.sp, fontWeight = FontWeight.Medium),
    titleSmall = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.Medium),
    bodyLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 16.sp, lineHeight = 26.sp, fontWeight = FontWeight.Normal),
    bodyMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 14.sp, lineHeight = 22.sp, fontWeight = FontWeight.Normal),
    bodySmall = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 12.sp, lineHeight = 18.sp, fontWeight = FontWeight.Normal),
    labelLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.Medium),
    labelMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 12.sp, lineHeight = 18.sp, fontWeight = FontWeight.Medium),
    labelSmall = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 11.sp, lineHeight = 16.sp, fontWeight = FontWeight.Medium),
)

@Composable
internal fun AgentRemoteTheme(content: @Composable () -> Unit) {
    val darkTheme = isSystemInDarkTheme()
    CompositionLocalProvider(
        LocalRemoteSemanticColors provides if (darkTheme) DarkSemanticColors else LightSemanticColors,
    ) {
        MaterialTheme(
            colorScheme = if (darkTheme) DarkColors else LightColors,
            typography = AgentRemoteTypography,
            content = content,
        )
    }
}
