plugins {
    `java-library`
}

val repoRoot = layout.projectDirectory.dir("../..")
val coreJar = repoRoot.file("lume/core/build/libs/lume-core.jar")
val dbSource = layout.projectDirectory.file("src/main/lume/lume/db/Db.lum")
val dbLumeSources = layout.projectDirectory.dir("src/main/lume")
val jdbcJava = layout.projectDirectory.dir("src/main/java")
val apiJava = layout.projectDirectory.dir("src/main/java-api")
val jdbcClasses = layout.buildDirectory.dir("classes/java/jdbc")
val generatedLumeJava = layout.buildDirectory.dir("generated/sources/lume/java")
val lumeCompilerSources = repoRoot.dir("rust/crates/lume/src")
val lumeCompilerManifest = repoRoot.file("rust/Cargo.toml")
val lumeCompilerBinary = repoRoot.file("rust/target/debug/lume")
val lumeExecutableOverride = providers.environmentVariable("LUME")
val currentGradleExecutable = providers.provider {
    val executableName = if (System.getProperty("os.name").lowercase().contains("windows")) {
        "gradle.bat"
    } else {
        "gradle"
    }
    gradle.gradleHomeDir?.resolve("bin")?.resolve(executableName)?.absolutePath ?: "gradle"
}
val gradleExecutable = providers.environmentVariable("GRADLE").orElse(currentGradleExecutable)

dependencies {
    api(files(coreJar))
}

sourceSets {
    main {
        java.srcDir(generatedLumeJava)
        java.srcDir(apiJava)
    }
}

val buildLocalLumeCore = tasks.register<Exec>("buildLocalLumeCore") {
    description = "Builds the repo-local Lume core jar used by Lume DB."
    group = "build"

    commandLine(
        gradleExecutable.get(),
        "-p",
        repoRoot.dir("lume/core").asFile.absolutePath,
        "jar"
    )

    inputs.file(repoRoot.file("lume/core/build.gradle.kts"))
    inputs.file(repoRoot.file("lume/core/settings.gradle.kts"))
    inputs.files(fileTree(repoRoot.dir("lume/core/src")))
    outputs.file(coreJar)
}

val cleanGeneratedLumeJava = tasks.register("cleanGeneratedLumeJava") {
    outputs.dir(generatedLumeJava)

    doLast {
        val outputDir = generatedLumeJava.get().asFile
        outputDir.deleteRecursively()
        outputDir.mkdirs()
    }
}

val buildLumeCompiler = tasks.register<Exec>("buildLumeCompiler") {
    description = "Builds the repo-local Lume compiler unless LUME points to an installed compiler."
    group = "build"
    onlyIf { !lumeExecutableOverride.isPresent }

    inputs.file(lumeCompilerManifest)
    inputs.files(fileTree(lumeCompilerSources))
    outputs.file(lumeCompilerBinary)

    commandLine(
        "cargo",
        "build",
        "--manifest-path",
        lumeCompilerManifest.asFile.absolutePath,
        "-p",
        "lume"
    )
}

val compileJdbcJava = tasks.register<JavaCompile>("compileJdbcJava") {
    description = "Compiles the tiny JDBC adapter used by the Lume DB API."
    group = "build"
    dependsOn(buildLocalLumeCore)

    source(fileTree(jdbcJava))
    classpath = files(coreJar)
    destinationDirectory.set(jdbcClasses)
}

val generateDbJava = tasks.register<Exec>("generateDbJava") {
    description = "Generates Java sources for the Lume DB API."
    group = "build"
    dependsOn(buildLumeCompiler, compileJdbcJava, cleanGeneratedLumeJava)

    inputs.files(fileTree(dbLumeSources))
    inputs.files(fileTree(jdbcJava))
    inputs.files(fileTree(lumeCompilerSources))
    inputs.property("lumeExecutable", lumeExecutableOverride.orNull ?: lumeCompilerBinary.asFile.absolutePath)
    if (!lumeExecutableOverride.isPresent) {
        inputs.file(lumeCompilerBinary)
    }
    outputs.dir(generatedLumeJava)

    doFirst {
        val outputDir = generatedLumeJava.get().asFile
        val lumeExecutable = lumeExecutableOverride.orNull ?: lumeCompilerBinary.asFile.absolutePath
        val generationClasspath = files(
            coreJar,
            jdbcClasses
        ).asPath

        commandLine(
            lumeExecutable,
            "gen",
            dbSource.asFile.absolutePath,
            "--out",
            outputDir.absolutePath,
            "--classpath",
            generationClasspath
        )
    }
}

tasks.named<JavaCompile>("compileJava") {
    dependsOn(generateDbJava, compileJdbcJava)
    classpath += files(jdbcClasses)
}

tasks.named<Jar>("jar") {
    archiveBaseName.set("lume-db")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    from(jdbcClasses)
    from({
        configurations.runtimeClasspath.get().map { dependency ->
            if (dependency.isDirectory) dependency else zipTree(dependency)
        }
    })
}
