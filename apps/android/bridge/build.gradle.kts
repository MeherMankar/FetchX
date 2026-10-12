plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "dev.mydownloader.bridge"
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
    // UniFFI 0.29 requires JNA for calling native methods on Android
    implementation(libs.jna)

    testImplementation(libs.junit)
    testImplementation(libs.kotlin.test)
}
