plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.chaquopy)
}

val releaseSigningVariables = listOf(
    "ANDROID_KEYSTORE_PATH",
    "ANDROID_KEY_ALIAS",
    "ANDROID_KEY_PASSWORD",
    "ANDROID_STORE_PASSWORD"
)
val configuredReleaseSigningVariables =
    releaseSigningVariables.count { !System.getenv(it).isNullOrBlank() }
require(configuredReleaseSigningVariables == 0 || configuredReleaseSigningVariables == releaseSigningVariables.size) {
    "Android release signing requires ANDROID_KEYSTORE_PATH, ANDROID_KEY_ALIAS, ANDROID_KEY_PASSWORD, and ANDROID_STORE_PASSWORD."
}
val ciArm64Only = providers.gradleProperty("ciArm64Only").orNull == "true"
val appVersion = providers.gradleProperty("appVersion").orElse("0.0.0-dev").get().removePrefix("v")
val appVersionCode = providers.gradleProperty("appVersionCode").orElse("1").get().toInt()
android {
    namespace = "dev.fetchx.app"
    compileSdk {
        version = release(37) {
            minorApiLevel = 2
        }
    }

    defaultConfig {
        applicationId = "dev.fetchx.app"
        minSdk = 26
        targetSdk = 35
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        versionCode = appVersionCode
        versionName = appVersion
        ndk {
            abiFilters += if (ciArm64Only) {
                listOf("arm64-v8a")
            } else {
                listOf("arm64-v8a", "x86_64")
            }
        }
    }

    signingConfigs {
        create("release") {
            if (configuredReleaseSigningVariables == releaseSigningVariables.size) {
                storeFile = file(System.getenv("ANDROID_KEYSTORE_PATH"))
                storePassword = System.getenv("ANDROID_STORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            if (configuredReleaseSigningVariables == releaseSigningVariables.size) {
                signingConfig = signingConfigs.getByName("release")
            }
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
        debug {
            applicationIdSuffix = ".debug"
            isDebuggable = true
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlin {
        compilerOptions {
            jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
        }
    }

    buildFeatures {
        compose = true
    }

    packaging {
        jniLibs {
            useLegacyPackaging = true
            keepDebugSymbols += "**/libfetchx_ffmpeg.so"
        }
    }
}

chaquopy {
    defaultConfig {
        version = "3.13"
        buildPython = if (System.getenv("pythonLocation") != null)
            listOf("${System.getenv("pythonLocation")}/bin/python3.13")
        else
            null
        pip {
            install("yt-dlp==2026.8.19")
        }
    }
}

dependencies {
    implementation(project(":core"))
    implementation(project(":downloads"))
    implementation(project(":bridge"))
    implementation(project(":browser"))

    implementation(platform(libs.compose.bom))
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons.extended)
    implementation(libs.kotlinx.coroutines.android)

    testImplementation(libs.junit)
    testImplementation(libs.kotlin.test)
    testImplementation(libs.kotlinx.coroutines.test)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.ext.junit)
}
