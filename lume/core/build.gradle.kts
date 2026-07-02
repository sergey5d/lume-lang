plugins {
    java
}

dependencies {
    implementation("io.javalin:javalin:6.7.0")
    runtimeOnly("org.slf4j:slf4j-simple:2.0.17")
}

val repoRoot = layout.projectDirectory.dir("../..")
val lumeSource = layout.projectDirectory.file("src/main/lume/lume/core/http/HttpServer.lum")
val runtimeJava = layout.projectDirectory.dir("src/main/java")
val runtimeClasses = layout.buildDirectory.dir("runtime-classes")
val generatedLumeJava = layout.buildDirectory.dir("generated/sources/lume/java")
val lumeCompilerSources = repoRoot.dir("rust/crates/lume/src")

sourceSets {
    main {
        java.srcDir(runtimeJava)
        java.srcDir(generatedLumeJava)
    }
}

val compileRuntimeJava = tasks.register<JavaCompile>("compileRuntimeJava") {
    description = "Compiles Java runtime substrate so Lume can inspect it during core generation."
    source = fileTree(runtimeJava)
    classpath = configurations.compileClasspath.get()
    destinationDirectory.set(runtimeClasses)
}

val generateLumeJava = tasks.register<Exec>("generateLumeJava") {
    description = "Generates Java sources for Lume core libraries."
    group = "build"
    dependsOn(compileRuntimeJava)

    inputs.file(lumeSource)
    inputs.files(fileTree(runtimeJava))
    inputs.files(fileTree(lumeCompilerSources))
    outputs.dir(generatedLumeJava)

    doFirst {
        val outputDir = generatedLumeJava.get().asFile
        outputDir.deleteRecursively()
        outputDir.mkdirs()

        val lumeClasspath = files(
            runtimeClasses.get().asFile,
            configurations.compileClasspath.get()
        ).asPath

        commandLine(
            "cargo",
            "run",
            "--manifest-path",
            repoRoot.file("rust/Cargo.toml").asFile.absolutePath,
            "-p",
            "lume",
            "--",
            "java",
            lumeSource.asFile.absolutePath,
            "--out",
            outputDir.absolutePath,
            "--classpath",
            lumeClasspath
        )
    }
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
