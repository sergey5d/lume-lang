plugins {
    id("lume.java-application")
}

val repoRoot = layout.projectDirectory.dir("../..")
val localLume = repoRoot.file("rust/target/debug/lume")
val lumeCoreJar = repoRoot.file("lume/core/build/libs/lume-core.jar")
val lumeHttpJar = repoRoot.file("lume/http/build/libs/lume-http.jar")
val selectedLumeExecutable = providers.environmentVariable("LUME")
    .orElse(localLume.asFile.absolutePath)
val gradleExecutable = providers.environmentVariable("GRADLE")
    .orElse("gradle")

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

val buildLocalLumeHttp = tasks.register<Exec>("buildLocalLumeHttp") {
    description = "Builds the repo-local Lume HTTP jar used by this checkout sample."
    group = "build"
    dependsOn(buildLocalLumeCore)

    commandLine(
        gradleExecutable.get(),
        "-p",
        repoRoot.dir("lume/http").asFile.absolutePath,
        "jar"
    )

    inputs.file(repoRoot.file("lume/http/build.gradle.kts"))
    inputs.file(repoRoot.file("lume/http/settings.gradle.kts"))
    inputs.files(fileTree(repoRoot.dir("lume/http/src")))
    inputs.file(lumeCoreJar)
    outputs.file(lumeHttpJar)
}

lumeJava {
    source.set(layout.projectDirectory.file("src/main/lume/service.lum"))
    mainClass.set("examples.java_gradle_rest.Java_gradle_restMain")

    // The plugin itself defaults to installed `lume`; this sample points at the
    // checkout compiler unless LUME overrides it.
    lumeExecutable.set(selectedLumeExecutable)
    runtimeClasspath.from(lumeCoreJar, lumeHttpJar)
}

tasks.named("generateLumeJava") {
    dependsOn(buildLocalLumeCore, buildLocalLumeHttp)
    inputs.files(lumeCoreJar, lumeHttpJar)
}

tasks.named("compileJava") {
    dependsOn(buildLocalLumeCore, buildLocalLumeHttp)
}
