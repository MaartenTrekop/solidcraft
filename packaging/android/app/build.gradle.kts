plugins {
    id("com.android.application")
}

android {
    // Keep in step with PACKAGE in apps/solidcraft-android/src/storage.rs.
    namespace = "io.github.maartentrekop.solidcraft"
    compileSdk = 35

    defaultConfig {
        applicationId = "io.github.maartentrekop.solidcraft"
        // Android 11: "All files access" (MANAGE_EXTERNAL_STORAGE) is how the app reaches files.
        minSdk = 30
        // 34, not 35: from 35 on Android draws apps under the status and navigation bars, which
        // the app's layout doesn't leave room for yet.
        targetSdk = 34
        versionCode = (System.getenv("SOLIDCRAFT_VERSION_CODE") ?: "1").toInt()
        versionName = System.getenv("SOLIDCRAFT_VERSION_NAME") ?: "dev"
        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    signingConfigs {
        // A fixed key, so each new build installs over the last one. It is in the repository and
        // only fit for installing your own builds; see README.md.
        create("sideload") {
            storeFile = rootProject.file("sideload.keystore")
            storeType = "PKCS12"
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("sideload")
        }
        debug {
            signingConfig = signingConfigs.getByName("sideload")
        }
    }

    packaging {
        jniLibs {
            // Stored uncompressed and page aligned: Android maps the library straight from the APK.
            useLegacyPackaging = false
        }
    }

    lint {
        // No Java or Kotlin code to check; the manifest asks for all files access on purpose.
        checkReleaseBuilds = false
        abortOnError = false
    }
}
