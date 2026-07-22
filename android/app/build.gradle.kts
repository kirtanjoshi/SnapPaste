plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.snappaste.app"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.snappaste.app"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    // Kotlin coroutines (used by the service/observer layer)
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1")

    // AndroidX Core
    implementation("androidx.core:core-ktx:1.13.1")

    // EncryptedSharedPreferences for secure pairing token storage
    implementation("androidx.security:security-crypto:1.1.0-alpha06")

    // Unit tests
    testImplementation("junit:junit:4.13.2")

    // Instrumented tests
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.1")
    androidTestImplementation("androidx.test:rules:1.6.1")
}
