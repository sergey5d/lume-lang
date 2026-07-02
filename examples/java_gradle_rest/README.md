# Lume Gradle REST Example

This sample shows the intended Java backend shape: application code is Lume,
route discovery is Lume, and Java is only the low-level HTTP substrate.

- Lume service code lives in `src/main/lume`.
- Core Lume libraries live under `../../lume/core`.
- `lume-core.jar` contains the Lume runtime, metadata descriptors, and the
  Lume-written `HttpServer`.
- Gradle generates Java from Lume before compiling.
- `gradle build` produces a runnable jar.

## Build Flow

The important tasks are:

- `buildLumeCore`: builds `../../lume/core/build/libs/lume-core.jar`.
- `generateLumeJava`: runs `cargo run -p lume -- gen ...`; `lume-core.jar`
  is available automatically to generation.
- `compileJava`: compiles the generated application Java.
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

## HTTP Boundary

The sample imports the Lume core HTTP API as:

```txt
use lume/core/http/{HttpServer, Controller, GET, POST}
```

The controller is pure Lume:

```txt
@Controller { path: "/api" }
class GreetingController {
    @GET { path: "/health" }
    def health() Str = "ok"
}
```

`HttpServer.addController(controller)` is written in Lume. It inspects Lume
annotations through runtime metadata and registers discovered routes on a tiny
Java `JavalinBackend`. The Java backend knows how to expose `get`, `post`, and
`run`; it does not know application paths or controllers.
