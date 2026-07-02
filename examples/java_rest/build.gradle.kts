plugins {
    id("lume.java-application")
}

lumeJava {
    source.set(layout.projectDirectory.file("service.lum"))
    mainClass.set("examples.java_rest.Java_restMain")
}
