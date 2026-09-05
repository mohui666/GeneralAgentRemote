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
    primary = Color(0xFF315DD8),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFEAF0FF),
    onPrimaryContainer = Color(0xFF1D2939),
    secondary = Color(0xFF667085),
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFEAF0FF),
    onSecondaryContainer = Color(0xFF1D2939),
    tertiary = Color(0xFF667085),
    onTertiary = Color.White,
    tertiaryContainer = Color(0xFFEDF1F7),
    onTertiaryContainer = Color(0xFF1D2939),
    background = Color(0xFFF8FAFC),
    onBackground = Color(0xFF1D2939),
    surface = Color.White,
    onSurface = Color(0xFF1D2939),
    surfaceVariant = Color(0xFFF0F3F8),
    onSurfaceVariant = Color(0xFF667085),
    surfaceTint = Color(0xFF1D2939),
    surfaceBright = Color.White,
    surfaceDim = Color(0xFFEAF0FF),
    surfaceContainerLowest = Color.White,
    surfaceContainerLow = Color(0xFFF8FAFC),
    surfaceContainer = Color(0xFFF0F3F8),
    surfaceContainerHigh = Color(0xFFEDF1F7),
    surfaceContainerHighest = Color(0xFFEAF0FF),
    outline = Color(0xFFE1E7EF),
    outlineVariant = Color(0xFFE1E7EF),
    inverseSurface = Color(0xFF315DD8),
    inverseOnSurface = Color(0xFFF0F3F8),
    inversePrimary = Color(0xFFB7CAFF),
    error = Color(0xFF9C3938),
    onError = Color.White,
    errorContainer = Color(0xFFFBEFED),
    onErrorContainer = Color(0xFF9C3938),
    scrim = Color(0xFF202226),
)

private val DarkColors = darkColorScheme(
    primary = Color(0xFFB7CAFF),
    onPrimary = Color(0xFF142038),
    primaryContainer = Color(0xFF253550),
    onPrimaryContainer = Color(0xFFE4EAF4),
    secondary = Color(0xFF9DAAC0),
    onSecondary = Color(0xFF142038),
    secondaryContainer = Color(0xFF253550),
    onSecondaryContainer = Color(0xFFE4EAF4),
    tertiary = Color(0xFF9DAAC0),
    onTertiary = Color(0xFF142038),
    tertiaryContainer = Color(0xFF1F2A3D),
    onTertiaryContainer = Color(0xFFE4EAF4),
    background = Color(0xFF121A28),
    onBackground = Color(0xFFE4EAF4),
    surface = Color(0xFF182233),
    onSurface = Color(0xFFE4EAF4),
    surfaceVariant = Color(0xFF1F2A3D),
    onSurfaceVariant = Color(0xFF9DAAC0),
    surfaceTint = Color(0xFFE4EAF4),
    surfaceBright = Color(0xFF314058),
    surfaceDim = Color(0xFF101724),
    surfaceContainerLowest = Color(0xFF101724),
    surfaceContainerLow = Color(0xFF121A28),
    surfaceContainer = Color(0xFF182233),
    surfaceContainerHigh = Color(0xFF1F2A3D),
    surfaceContainerHighest = Color(0xFF253550),
    outline = Color(0xFF2D3B50),
    outlineVariant = Color(0xFF2D3B50),
    inverseSurface = Color(0xFFB7CAFF),
    inverseOnSurface = Color(0xFF142038),
    inversePrimary = Color(0xFF315DD8),
    error = Color(0xFFE7A6A1),
    onError = Color(0xFF352827),
    errorContainer = Color(0xFF352827),
    onErrorContainer = Color(0xFFE7A6A1),
    scrim = Color(0xFF090A0B),
)

private data class RemoteSemanticColors(val accent: Color, val success: Color, val warning: Color)

private val LightSemanticColors = RemoteSemanticColors(
    accent = Color(0xFF315DD8),
    success = Color(0xFF168565),
    warning = Color(0xFF8B632D),
)
private val DarkSemanticColors = RemoteSemanticColors(
    accent = Color(0xFFADC5FF),
    success = Color(0xFF68D4AD),
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
    bodyLarge = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 15.sp, lineHeight = 23.sp, fontWeight = FontWeight.Normal),
    bodyMedium = TextStyle(fontFamily = FontFamily.SansSerif, fontSize = 14.sp, lineHeight = 21.sp, fontWeight = FontWeight.Normal),
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
