plugins {
    application
    java
}

dependencies {
    implementation(files(layout.projectDirectory.dir("../..").file("lume/core/build/libs/lume-core.jar")))
}

val repoRoot = layout.projectDirectory.dir("../..")
val lumeSource = layout.projectDirectory.file("src/main/lume/service.lum")
val generatedLumeJava = layout.buildDirectory.dir("generated/sources/lume/java")
val lumeCoreJar = repoRoot.file("lume/core/build/libs/lume-core.jar")
val lumeCompilerSources = repoRoot.dir("rust/crates/lume/src")
val lumeCompilerManifest = repoRoot.file("rust/Cargo.toml")
val lumeCompilerBinary = repoRoot.file("rust/target/debug/lume")
val lumeExecutableOverride = providers.environmentVariable("LUME")
val gradleExecutable = providers.environmentVariable("GRADLE").orElse("gradle")

application {
    mainClass.set("examples.java_gradle_rest.Java_gradle_restMain")
}

sourceSets {
    main {
        java.srcDir(generatedLumeJava)
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

val buildLumeCore = tasks.register<Exec>("buildLumeCore") {
    description = "Builds lume-core.jar for app generation and compilation."
    group = "build"
    dependsOn(buildLumeCompiler)

    inputs.files(fileTree(repoRoot.dir("lume/core")))
    inputs.file(lumeCompilerManifest)
    inputs.files(fileTree(lumeCompilerSources))
    outputs.file(lumeCoreJar)

    commandLine(
        gradleExecutable.get(),
        "-p",
        repoRoot.dir("lume/core").asFile.absolutePath,
        "jar",
        "--no-daemon"
    )
}

val generateLumeJava = tasks.register<Exec>("generateLumeJava") {
    description = "Generates Java sources from Lume sources."
    group = "build"
    dependsOn(buildLumeCore)

    inputs.file(lumeSource)
    inputs.file(lumeCoreJar)
    inputs.files(fileTree(lumeCompilerSources))
    inputs.property("lumeExecutable", lumeExecutableOverride.orNull ?: lumeCompilerBinary.asFile.absolutePath)
    if (!lumeExecutableOverride.isPresent) {
        inputs.file(lumeCompilerBinary)
    }
    outputs.dir(generatedLumeJava)

    doFirst {
        val outputDir = generatedLumeJava.get().asFile
        val lumeExecutable = lumeExecutableOverride.orNull ?: lumeCompilerBinary.asFile.absolutePath
        outputDir.deleteRecursively()
        outputDir.mkdirs()

        commandLine(
            lumeExecutable,
            "gen",
            lumeSource.asFile.absolutePath,
            "--out",
            outputDir.absolutePath
        )
    }
}

tasks.named<JavaCompile>("compileJava") {
    dependsOn(generateLumeJava)
}

tasks.named<Jar>("jar") {
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    manifest {
        attributes["Main-Class"] = application.mainClass.get()
    }
    from({
        configurations.runtimeClasspath.get().map { dependency ->
            if (dependency.isDirectory) dependency else zipTree(dependency)
        }
    })
}
