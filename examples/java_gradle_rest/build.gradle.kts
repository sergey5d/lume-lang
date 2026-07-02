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
val localGradle = file("/tmp/gradle-9.6.1/bin/gradle")
val gradleExecutable = providers.environmentVariable("GRADLE")
    .orElse(if (localGradle.exists()) localGradle.absolutePath else "gradle")

application {
    mainClass.set("examples.java_gradle_rest.Java_gradle_restMain")
}

sourceSets {
    main {
        java.srcDir(generatedLumeJava)
    }
}

val buildLumeCore = tasks.register<Exec>("buildLumeCore") {
    description = "Builds lume-core.jar for app generation and compilation."
    group = "build"

    inputs.files(fileTree(repoRoot.dir("lume/core")))
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
    outputs.dir(generatedLumeJava)

    doFirst {
        val outputDir = generatedLumeJava.get().asFile
        outputDir.deleteRecursively()
        outputDir.mkdirs()

        commandLine(
            "cargo",
            "run",
            "--manifest-path",
            repoRoot.file("rust/Cargo.toml").asFile.absolutePath,
            "-p",
            "lume",
            "--",
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
