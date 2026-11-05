plugins {
    id("com.android.application")
}

android {
    namespace = "dev.fetchx.durabilityfixture"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.fetchx.durabilityfixture"
        minSdk = 23
        targetSdk = 28
        versionCode = 1
        versionName = "1.0"
    }
}
