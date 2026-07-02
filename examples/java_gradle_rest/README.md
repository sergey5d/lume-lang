# Lume Gradle REST Example

This is the Gradle-shaped version of the Java backend flow:

- Lume service code lives in `src/main/lume`.
- Java bridge code lives in `src/main/java`.
- Gradle resolves Java dependencies, including Javalin.
- Gradle generates Java from Lume before compiling.
- `gradle build` produces a runnable jar.

## Build Flow

The important tasks are:

- `compileBridgeJava`: compiles `src/main/java` into `build/bridge-classes`.
- `generateLumeJava`: runs `cargo run -p lume -- java ...` with the bridge classes and Gradle dependencies on the Lume Java classpath.
- `compileJava`: compiles bridge Java, generated Lume Java, and `java_runtime` sources together.
- `jar`: packages a runnable fat jar with Java dependencies.

## Commands

From this folder:

```bash
gradle build
gradle run
```

Or run the jar:

```bash
java -jar build/libs/lume-java-gradle-rest.jar
```

Then test:

```bash
curl http://localhost:7070/api/health
curl http://localhost:7070/api/hello
curl -X POST http://localhost:7070/api/echo -d 'from curl'
```

## Current Limitation

The Lume annotations are real syntax and are typechecked, but this bridge still
mirrors route metadata manually in Java. The next backend step is to emit
annotation payloads or a generated route registry so the Java bridge can discover
controllers without hardcoded route registration.
