plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

val repositoryRoot = rootProject.projectDir.resolve("../..").canonicalFile
val bindingOutputDirectory = layout.buildDirectory.dir("generated/uniffi/kotlin")
val generatedBinding = bindingOutputDirectory.map {
    it.file("dev/fetchx/bridge/uniffi/android_bridge.kt")
}
val generateUniFfiKotlinBindings = tasks.register<Exec>("generateUniFfiKotlinBindings") {
    val udlFile = repositoryRoot.resolve("crates/android-bridge/uniffi/android_bridge.udl")
    val configFile = repositoryRoot.resolve("crates/android-bridge/uniffi.toml")
    val bridgeManifest = repositoryRoot.resolve("crates/android-bridge/Cargo.toml")

    inputs.files(
        udlFile,
        configFile,
        bridgeManifest,
        repositoryRoot.resolve("Cargo.toml"),
        repositoryRoot.resolve("Cargo.lock"),
    )
    outputs.file(generatedBinding)
    workingDir(repositoryRoot)
    commandLine(
        "cargo",
        "run",
        "--locked",
        "-p",
        "fetchx-android-bridge",
        "--features",
        "uniffi-cli",
        "--bin",
        "uniffi-bindgen",
        "--",
        "generate",
        udlFile.relativeTo(repositoryRoot).path,
        "--language",
        "kotlin",
        "--no-format",
        "--config",
        configFile.relativeTo(repositoryRoot).path,
        "--out-dir",
        bindingOutputDirectory.get().asFile.absolutePath,
    )
}

android {
    namespace = "dev.fetchx.bridge"
    compileSdk = 35

    defaultConfig {
        minSdk = 26
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
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
            java.srcDir(bindingOutputDirectory)
        }
    }
}

tasks.named("preBuild") {
    dependsOn(generateUniFfiKotlinBindings)
}

dependencies {
    // :bridge implements the DownloadRepository interface from :core
    implementation(project(":core"))
    implementation(libs.kotlinx.coroutines.android)
    // UniFFI 0.29 uses JNA for FFI dispatch on Android.
    // The @aar classifier is required — it bundles libjnidispatch.so for each ABI.
    // Without it the plain JVM jar is used which lacks the native dispatch library,
    // causing UnsatisfiedLinkError at runtime on device.
    implementation("net.java.dev.jna:jna:5.17.0@aar")

    testImplementation(libs.junit)
    testImplementation(libs.kotlin.test)
}
