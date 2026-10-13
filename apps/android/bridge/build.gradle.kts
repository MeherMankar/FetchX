plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "dev.fetchx.bridge"
    compileSdk = 35

    defaultConfig {
        minSdk = 26
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    sourceSets {
        getByName("main") {
            // Pre-built .so files land here; directories are created but .so files
            // are not checked in — only .gitkeep placeholders.
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }
}

dependencies {
    // :bridge implements the DownloadRepository interface from :core
    implementation(project(":core"))
    implementation(libs.kotlinx.coroutines.android)
    // UniFFI 0.29 uses JNA for FFI dispatch on Android.
    // The @aar classifier is required — it bundles libjnidispatch.so for each ABI.
    // Without it the plain JVM jar is used which lacks the native dispatch library,
    // causing UnsatisfiedLinkError at runtime on device.
    implementation("net.java.dev.jna:jna:5.15.0@aar")

    testImplementation(libs.junit)
    testImplementation(libs.kotlin.test)
}

