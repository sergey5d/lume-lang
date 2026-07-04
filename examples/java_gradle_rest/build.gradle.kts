plugins {
    id("lume.java-application")
}

val repoRoot = layout.projectDirectory.dir("../..")
val localLume = repoRoot.file("rust/target/debug/lume")
val lumeCoreJar = repoRoot.file("lume/core/build/libs/lume-core.jar")
val lumeHttpJavalinJar = repoRoot.file("lume/http/javalin/build/libs/lume-http-javalin.jar")
val selectedLumeExecutable = providers.environmentVariable("LUME")
    .orElse(localLume.asFile.absolutePath)
val currentGradleExecutable = providers.provider {
    val executableName = if (System.getProperty("os.name").lowercase().contains("windows")) {
        "gradle.bat"
    } else {
        "gradle"
    }
    gradle.gradleHomeDir?.resolve("bin")?.resolve(executableName)?.absolutePath ?: "gradle"
}
val gradleExecutable = providers.environmentVariable("GRADLE").orElse(currentGradleExecutable)

val buildLocalLumeCore = tasks.register<Exec>("buildLocalLumeCore") {
    description = "Builds the repo-local Lume core jar used by this checkout sample."
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
    outputs.file(lumeCoreJar)
}

val buildLocalLumeHttpJavalin = tasks.register<Exec>("buildLocalLumeHttpJavalin") {
    description = "Builds the repo-local Lume HTTP Javalin jar used by this checkout sample."
    group = "build"
    dependsOn(buildLocalLumeCore)

    commandLine(
        gradleExecutable.get(),
        "-p",
        repoRoot.dir("lume/http/javalin").asFile.absolutePath,
        "jar"
    )

    inputs.file(repoRoot.file("lume/http/javalin/build.gradle.kts"))
    inputs.file(repoRoot.file("lume/http/javalin/settings.gradle.kts"))
    inputs.files(fileTree(repoRoot.dir("lume/http/javalin/src")))
    inputs.file(lumeCoreJar)
    outputs.file(lumeHttpJavalinJar)
}

lumeJava {
    source.set(layout.projectDirectory.file("src/main/lume/service.lum"))
    mainClass.set("examples.java_gradle_rest.Java_gradle_restMain")

    // The plugin itself defaults to installed `lume`; this sample points at the
    // checkout compiler unless LUME overrides it.
    lumeExecutable.set(selectedLumeExecutable)
    runtimeClasspath.from(lumeCoreJar, lumeHttpJavalinJar)
}

tasks.named("generateLumeJava") {
    dependsOn(buildLocalLumeCore, buildLocalLumeHttpJavalin)
    inputs.files(lumeCoreJar, lumeHttpJavalinJar)
}

tasks.named("compileJava") {
    dependsOn(buildLocalLumeCore, buildLocalLumeHttpJavalin)
}
