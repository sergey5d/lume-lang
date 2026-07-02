plugins {
    java
}

dependencies {
    implementation("io.javalin:javalin:6.7.0")
    runtimeOnly("org.slf4j:slf4j-simple:2.0.17")
}

val repoRoot = layout.projectDirectory.dir("../..")
val optionSource = layout.projectDirectory.file("src/main/lume/lume/core/Option.lum")
val resultSource = layout.projectDirectory.file("src/main/lume/lume/core/Result.lum")
val eitherSource = layout.projectDirectory.file("src/main/lume/lume/core/Either.lum")
val httpSource = layout.projectDirectory.file("src/main/lume/lume/core/http/HttpServer.lum")
val lumeSources = listOf(optionSource, resultSource, eitherSource, httpSource)
val runtimeJava = layout.projectDirectory.dir("src/main/java")
val javaStubs = layout.projectDirectory.dir("src/main/java-stubs")
val runtimeClasses = layout.buildDirectory.dir("runtime-classes")
val generatedLumeJava = layout.buildDirectory.dir("generated/sources/lume/java")
val lumeCompilerSources = repoRoot.dir("rust/crates/lume/src")
val lumeCompilerManifest = repoRoot.file("rust/Cargo.toml")
val lumeCompilerBinary = repoRoot.file("rust/target/debug/lume")
val lumeExecutableOverride = providers.environmentVariable("LUME")

sourceSets {
    main {
        java.srcDir(runtimeJava)
        java.srcDir(generatedLumeJava)
    }
}

val compileRuntimeJava = tasks.register<JavaCompile>("compileRuntimeJava") {
    description = "Compiles tiny Java stubs so Lume can inspect external classes during core generation."
    source = fileTree(javaStubs)
    classpath = configurations.compileClasspath.get()
    destinationDirectory.set(runtimeClasses)
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

fun Exec.configureLumeJavaGeneration(source: org.gradle.api.file.RegularFile) {
    dependsOn(buildLumeCompiler)
    inputs.file(source)
    inputs.files(fileTree(javaStubs))
    inputs.files(fileTree(lumeCompilerSources))
    inputs.property("lumeExecutable", lumeExecutableOverride.orNull ?: lumeCompilerBinary.asFile.absolutePath)
    if (!lumeExecutableOverride.isPresent) {
        inputs.file(lumeCompilerBinary)
    }
    outputs.dir(generatedLumeJava)

    doFirst {
        val outputDir = generatedLumeJava.get().asFile
        val lumeExecutable = lumeExecutableOverride.orNull ?: lumeCompilerBinary.asFile.absolutePath
        val lumeClasspath = files(
            runtimeClasses.get().asFile,
            configurations.compileClasspath.get()
        ).asPath

        commandLine(
            lumeExecutable,
            "gen",
            source.asFile.absolutePath,
            "--out",
            outputDir.absolutePath,
            "--classpath",
            lumeClasspath
        )
    }
}

val generateOptionJava = tasks.register<Exec>("generateOptionJava") {
    description = "Generates Java sources for Lume core Option."
    group = "build"
    dependsOn(compileRuntimeJava, cleanGeneratedLumeJava)
    configureLumeJavaGeneration(optionSource)
}

val generateResultJava = tasks.register<Exec>("generateResultJava") {
    description = "Generates Java sources for Lume core Result."
    group = "build"
    dependsOn(generateOptionJava)
    configureLumeJavaGeneration(resultSource)
}

val generateEitherJava = tasks.register<Exec>("generateEitherJava") {
    description = "Generates Java sources for Lume core Either."
    group = "build"
    dependsOn(generateResultJava)
    configureLumeJavaGeneration(eitherSource)
}

val generateHttpJava = tasks.register<Exec>("generateHttpJava") {
    description = "Generates Java sources for Lume core HTTP."
    group = "build"
    dependsOn(generateEitherJava)
    configureLumeJavaGeneration(httpSource)
}

val generateLumeJava = tasks.register("generateLumeJava") {
    description = "Generates Java sources for Lume core libraries."
    group = "build"
    dependsOn(generateHttpJava)
    inputs.files(lumeSources)
    outputs.dir(generatedLumeJava)
}

tasks.named<JavaCompile>("compileJava") {
    dependsOn(generateLumeJava)
}

tasks.named<Jar>("jar") {
    archiveBaseName.set("lume-core")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    from({
        configurations.runtimeClasspath.get().map { dependency ->
            if (dependency.isDirectory) dependency else zipTree(dependency)
        }
    })
}
