# FetchX ProGuard rules
# Add project-specific ProGuard rules here.

# Keep Kotlin metadata for reflection
-keepattributes *Annotation*, Signature, Exception

# UniFFI reaches Rust through JNA-generated proxies and reflected structure fields.
-keep class dev.fetchx.bridge.uniffi.** { *; }
-keep interface dev.fetchx.bridge.uniffi.** { *; }
-keep class com.sun.jna.** { *; }
-keep interface com.sun.jna.** { *; }

# JNA also contains optional desktop AWT integrations, which Android does not provide.
-dontwarn java.awt.Component
-dontwarn java.awt.GraphicsEnvironment
-dontwarn java.awt.HeadlessException
-dontwarn java.awt.Window

# Keep Compose runtime
-keep class androidx.compose.** { *; }

# SnakeYAML's optional JavaBeans introspection is unavailable on Android.
-dontwarn java.beans.BeanInfo
-dontwarn java.beans.FeatureDescriptor
-dontwarn java.beans.IntrospectionException
-dontwarn java.beans.Introspector
-dontwarn java.beans.PropertyDescriptor
