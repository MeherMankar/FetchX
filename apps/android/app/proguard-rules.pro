# FetchX ProGuard rules
# Add project-specific ProGuard rules here.

# Keep Kotlin metadata for reflection
-keepattributes *Annotation*, Signature, Exception

# Keep Compose runtime
-keep class androidx.compose.** { *; }

# SnakeYAML's optional JavaBeans introspection is unavailable on Android.
-dontwarn java.beans.BeanInfo
-dontwarn java.beans.FeatureDescriptor
-dontwarn java.beans.IntrospectionException
-dontwarn java.beans.Introspector
-dontwarn java.beans.PropertyDescriptor
