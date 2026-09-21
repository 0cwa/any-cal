import groovy.json.JsonSlurper

fun rustlsPlatformVerifierMavenRepository(): File {
    val repositoryRoot = rootDir.parentFile
    val metadata = providers.exec {
        workingDir = repositoryRoot
        commandLine(
            "cargo",
            "metadata",
            "--format-version",
            "1",
            "--filter-platform",
            "aarch64-linux-android",
            "--manifest-path",
            File(repositoryRoot, "crates/android-bridge/Cargo.toml").absolutePath,
        )
    }.standardOutput.asText.get()
    val packages = ((JsonSlurper().parseText(metadata) as Map<*, *>) ["packages"] as List<*>)
    val manifestPath = packages
        .asSequence()
        .mapNotNull { it as? Map<*, *> }
        .first { it["name"] == "rustls-platform-verifier-android" }
        .get("manifest_path") as? String
        ?: error("rustls-platform-verifier-android manifest path is missing")
    return File(File(manifestPath).parentFile, "maven")
}

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
        maven {
            url = uri(rustlsPlatformVerifierMavenRepository())
            metadataSources {
                mavenPom()
                artifact()
            }
        }
        google()
        mavenCentral()
    }
}

rootProject.name = "any-cal-android"
include(":app")
