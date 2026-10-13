pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
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

rootProject.name = "MyDownloader"
include(":app")
include(":core")
include(":downloads")
include(":bridge")
include(":browser")
