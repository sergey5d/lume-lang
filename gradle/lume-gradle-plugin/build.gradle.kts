plugins {
    `java-gradle-plugin`
}

gradlePlugin {
    plugins {
        create("lumeJavaApplication") {
            id = "lume.java-application"
            implementationClass = "lume.gradle.LumeJavaApplicationPlugin"
        }
    }
}
