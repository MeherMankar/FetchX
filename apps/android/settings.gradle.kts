pluginManagement {
    resolutionStrategy {
        eachPlugin {
            if (requested.id.id == "com.chaquo.python") {
                useModule("com.chaquo.python:gradle:${requested.version}")
            }
        }
    }
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
        maven("https://chaquo.com/maven")
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // Mozilla Maven for GeckoView (used in Phase 20F)
        maven(url = "https://maven.mozilla.org/maven2")
    }
}

rootProject.name = "fetchx"
include(":app")
include(":core")
include(":downloads")
include(":bridge")
include(":browser")
