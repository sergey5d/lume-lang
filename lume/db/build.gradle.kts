plugins {
    `java-library`
}

val repoRoot = layout.projectDirectory.dir("../..")
val coreJar = repoRoot.file("lume/core/build/libs/lume-core.jar")
val gradleExecutable = providers.environmentVariable("GRADLE").orElse("gradle")

dependencies {
    api(files(coreJar))
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

tasks.named<JavaCompile>("compileJava") {
    dependsOn(buildLocalLumeCore)
}

tasks.named<Jar>("jar") {
    archiveBaseName.set("lume-db")
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    from({
        configurations.runtimeClasspath.get().map { dependency ->
            if (dependency.isDirectory) dependency else zipTree(dependency)
        }
    })
}
