# FetchX ProGuard rules
# Add project-specific ProGuard rules here.

# Keep Kotlin metadata for reflection
-keepattributes *Annotation*, Signature, Exception

# Keep Compose runtime
-keep class androidx.compose.** { *; }
