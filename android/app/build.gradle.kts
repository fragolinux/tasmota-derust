import java.io.File
import javax.inject.Inject

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// One version for the whole project: the workspace version in the root Cargo.toml.
val appVersion: String = Regex("""(?m)^version\s*=\s*"([^"]+)"""")
    .find(File(rootDir.parentFile, "Cargo.toml").readText())!!.groupValues[1]

/** 1.2.3 -> 10203 (and 1.2.3-rc1 -> 10203 too). */
fun versionCodeOf(v: String): Int {
    val (major, minor, patch) = v.substringBefore('-').split('.').map { it.toInt() }
    return major * 10_000 + minor * 100 + patch
}

android {
    namespace = "dev.derust.tasmota"
    compileSdk {
        version = release(37)
    }
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "dev.derust.tasmota"
        minSdk = 33
        targetSdk = 37
        versionCode = versionCodeOf(appVersion)
        versionName = appVersion

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

    }

    // Only the ABIs the Rust core is built for (drops JNA's mips/x86/armeabi copies).
    // x86_64 is for the emulator, so release APKs (real phones) leave it out.
    buildTypes {
        debug {
            ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
        }
        release {
            ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a") }
            optimization {
                enable = true
            }
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures {
        compose = true
    }
}

// ------------------------------------------------------------------ Rust core
//
// Builds crates/mobile with cargo-ndk for every ABI and generates its Kotlin bindings
// with UniFFI. Both outputs live under build/ and are added to the variant's sources.

/** Cargo binary: the `derust.cargo` Gradle property, else the usual install locations. */
fun findCargo(): String {
    (findProperty("derust.cargo") as String?)?.let { return it }
    val home = System.getProperty("user.home")
    return listOf("/opt/homebrew/opt/rustup/bin/cargo", "$home/.cargo/bin/cargo", "/usr/local/bin/cargo")
        .firstOrNull { File(it).canExecute() } ?: "cargo"
}

abstract class RustCoreTask @Inject constructor(private val exec: ExecOperations) : DefaultTask() {
    @get:Input abstract val cargo: Property<String>
    @get:Input abstract val workspaceDir: Property<String>
    @get:Input abstract val ndkDir: Property<String>
    @get:Input abstract val abis: ListProperty<String>
    @get:Input abstract val minSdk: Property<Int>

    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val rustSources: ConfigurableFileCollection

    @get:OutputDirectory abstract val jniDir: DirectoryProperty
    @get:OutputDirectory abstract val kotlinDir: DirectoryProperty

    @TaskAction
    fun build() {
        val cargoBin = File(cargo.get()).parentFile
        val home = System.getProperty("user.home")
        val path = listOfNotNull(cargoBin?.path, "$home/.cargo/bin", System.getenv("PATH")).joinToString(":")
        val env = mapOf("PATH" to path, "ANDROID_NDK_HOME" to ndkDir.get())
        val jni = jniDir.get().asFile.apply { deleteRecursively(); mkdirs() }

        // cargo-auditable (when installed) embeds the dependency list in the library, so the
        // released APK can be checked for vulnerable crates (scripts/scan-artifacts.sh).
        val auditable = path.split(":").any { File(it, "cargo-auditable").canExecute() }
        exec.exec {
            workingDir(workspaceDir.get())
            environment(env)
            commandLine(
                listOf(cargo.get()) + (if (auditable) listOf("auditable") else emptyList()) + listOf("ndk") +
                    abis.get().flatMap { listOf("-t", it) } +
                    listOf("-P", minSdk.get().toString(), "-o", jni.path,
                        "build", "--profile", "dist-mobile", "-p", "tasmota-derust-mobile")
            )
        }

        val kotlin = kotlinDir.get().asFile.apply { deleteRecursively(); mkdirs() }
        val lib = File(jni, "${abis.get().first()}/libtasmota_derust_mobile.so")
        exec.exec {
            workingDir(workspaceDir.get())
            environment(env)
            commandLine(
                cargo.get(), "run", "-q", "-p", "uniffi-bindgen", "--",
                "generate", "--library", lib.path, "--language", "kotlin",
                "--out-dir", kotlin.path, "--no-format",
            )
        }
    }
}

val repoRoot = rootDir.parentFile
val rustCore = tasks.register<RustCoreTask>("rustCore") {
    group = "build"
    description = "Builds the Rust core for Android and generates its Kotlin bindings."
    cargo.set(findCargo())
    workspaceDir.set(repoRoot.path)
    ndkDir.set(androidComponents.sdkComponents.ndkDirectory.map { it.asFile.path })
    abis.set(listOf("arm64-v8a", "armeabi-v7a", "x86_64"))
    minSdk.set(33)
    rustSources.from(
        File(repoRoot, "Cargo.toml"),
        File(repoRoot, "Cargo.lock"),
        fileTree(File(repoRoot, "crates")) { exclude("desktop/**", "web/**") },
    )
}

androidComponents {
    onVariants { variant ->
        variant.sources.jniLibs?.addGeneratedSourceDirectory(rustCore, RustCoreTask::jniDir)
        variant.sources.kotlin?.addGeneratedSourceDirectory(rustCore, RustCoreTask::kotlinDir)
    }
}

// The release build's libraries are pinned in app/gradle.lockfile (`make lock-android` to
// refresh it), which is also what the security scan reads.
configurations.matching { it.name == "releaseRuntimeClasspath" }.configureEach {
    resolutionStrategy.activateDependencyLocking()
}

dependencies {
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation("${libs.jna.get()}@aar")
    testImplementation(libs.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.espresso.core)
    androidTestImplementation(libs.androidx.junit)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
    debugImplementation(libs.androidx.compose.ui.tooling)
}
