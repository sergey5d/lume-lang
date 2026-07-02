plugins {
    application
    java
}

dependencies {
    implementation("io.javalin:javalin:6.7.0")
    runtimeOnly("org.slf4j:slf4j-simple:2.0.17")
}

val repoRoot = layout.projectDirectory.dir("../..")
val lumeSource = layout.projectDirectory.file("src/main/lume/service.lum")
val generatedLumeJava = layout.buildDirectory.dir("generated/sources/lume/java")
val bridgeClasses = layout.buildDirectory.dir("bridge-classes")

application {
    mainClass.set("examples.java_gradle_rest.Java_gradle_restMain")
}

sourceSets {
    main {
        java.srcDir(generatedLumeJava)
        java.srcDir(repoRoot.dir("java_runtime/src/main/java"))
    }
}

val compileBridgeJava = tasks.register<JavaCompile>("compileBridgeJava") {
    description = "Compiles Java bridge classes so Lume can inspect them during Java generation."
    source = fileTree("src/main/java")
    classpath = configurations.compileClasspath.get()
    destinationDirectory.set(bridgeClasses)
}

val generateLumeJava = tasks.register<Exec>("generateLumeJava") {
    description = "Generates Java sources from Lume sources."
    group = "build"
    dependsOn(compileBridgeJava)

    inputs.file(lumeSource)
    inputs.files(fileTree("src/main/java"))
    outputs.dir(generatedLumeJava)

    doFirst {
        val outputDir = generatedLumeJava.get().asFile
        outputDir.deleteRecursively()
        outputDir.mkdirs()

        val lumeClasspath = files(
            bridgeClasses.get().asFile,
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
