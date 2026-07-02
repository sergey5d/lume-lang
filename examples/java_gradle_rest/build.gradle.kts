plugins {
    id("lume.java-application")
}

lumeJava {
    source.set(layout.projectDirectory.file("src/main/lume/service.lum"))
    mainClass.set("examples.java_gradle_rest.Java_gradle_restMain")
}
