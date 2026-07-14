# Lume Gradle REST Example

This sample shows the intended Java backend shape: application code is Lume,
route discovery is Lume, and Java is only the low-level HTTP substrate.

- Lume service code lives in `src/main/lume`.
- The local `lume.java-application` Gradle plugin runs `lume gen` before Java
  compilation.
- The plugin defaults to an installed `lume` executable. This repo sample points
  at `../../rust/target/debug/lume` unless `LUME` is set.
- `lume-core.jar` and `lume-http-javalin.jar` are ordinary Java dependencies
  for compilation/runtime. This sample builds both repo-local jars before
  generating the app.
- `gradle build` produces a runnable jar.

## Build Flow

The important tasks are:

- `buildLocalLumeCore`: builds `../../lume/core/build/libs/lume-core.jar`.
- `buildLocalLumeHttpJavalin`: builds
  `../../lume/http/javalin/build/libs/lume-http-javalin.jar`.
- `generateLumeJava`: runs the Lume compiler as `lume gen ...`;
  the core and HTTP jars are passed to generation.
- `compileJava`: compiles the generated application Java.
- `jar`: packages a runnable fat jar with Java dependencies.

The reusable plugin does not know about this repository, Cargo, or
Lume jar locations. The repo-local bootstrap is in this example build file only.

## Commands

From this folder:

```bash
gradle build
gradle run
```

To use an installed compiler instead of the repo-local debug binary:

```bash
LUME=lume gradle build
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

## HTTP Boundary

The sample imports the Lume HTTP API as:

```txt
use lume/http/javalin/{HttpServer, Controller, Get, Post}
```

The controller is pure Lume:

```txt
@Controller { path: "/api" }
class GreetingController {
    @Get { path: "/health" }
    def health() Str = "ok"
}
```

`HttpServer.addController(controller)` is written in Lume. It inspects Lume
annotations through runtime metadata and registers discovered routes on a tiny
Java `JavalinBackend`. The Java backend knows how to expose `get`, `post`, and
the other HTTP verbs plus `run`; it does not know application paths or controllers.
