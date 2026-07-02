plugins {
    java
}

dependencies {
    implementation(files(layout.projectDirectory.file("../core/build/libs/lume-core.jar")))
    implementation("io.javalin:javalin:6.7.0")
    runtimeOnly("org.slf4j:slf4j-simple:2.0.17")
}

val repoRoot = layout.projectDirectory.dir("../..")
val httpSource = layout.projectDirectory.file("src/main/lume/lume/http/HttpServer.lum")
val generatedLumeJava = layout.buildDirectory.dir("generated/sources/lume/java")
val lumeCompilerSources = repoRoot.dir("rust/crates/lume/src")
val lumeCompilerBinary = repoRoot.file("rust/target/debug/lume")
val lumeExecutableOverride = providers.environmentVariable("LUME")
val gradleExecutable = providers.environmentVariable("GRADLE").orElse("gradle")
val coreJar = repoRoot.file("lume/core/build/libs/lume-core.jar")

sourceSets {
    main {
        java.srcDir(generatedLumeJava)
    }
}

val buildLocalLumeCore = tasks.register<Exec>("buildLocalLumeCore") {
    description = "Builds the repo-local Lume core jar used by Lume HTTP."
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

val generateHttpJava = tasks.register<Exec>("generateHttpJava") {
    description = "Generates Java sources for Lume HTTP."
    group = "build"
    dependsOn(buildLocalLumeCore, cleanGeneratedLumeJava)

    inputs.file(httpSource)
    inputs.file(coreJar)
    inputs.files(fileTree(lumeCompilerSources))
    inputs.property("lumeExecutable", lumeExecutableOverride.orNull ?: lumeCompilerBinary.asFile.absolutePath)
    if (!lumeExecutableOverride.isPresent) {
        inputs.file(lumeCompilerBinary)
    }
    outputs.dir(generatedLumeJava)

    doFirst {
        val outputDir = generatedLumeJava.get().asFile
        val lumeExecutable = lumeExecutableOverride.orNull ?: lumeCompilerBinary.asFile.absolutePath

        commandLine(
            lumeExecutable,
            "gen",
            httpSource.asFile.absolutePath,
            "--out",
            outputDir.absolutePath,
            "--classpath",
            configurations.compileClasspath.get().asPath
        )
    }
}

tasks.named<JavaCompile>("compileJava") {
    dependsOn(generateHttpJava)
}

tasks.named<Jar>("jar") {
    archiveBaseName.set("lume-http")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    from({
        configurations.runtimeClasspath.get().map { dependency ->
            if (dependency.isDirectory) dependency else zipTree(dependency)
        }
    })
}
