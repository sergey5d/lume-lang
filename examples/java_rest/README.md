# Java REST Example

This is a small Gradle-backed REST example. The Lume service lives directly in
`service.lum`, and the local `lume.java-application` Gradle plugin generates and
compiles Java before running.

The Lume side imports the core HTTP API and lets `HttpServer.addController`
inspect annotations:

```txt
use lume/core/http/{HttpServer, Controller, GET, POST}

@Controller { path: "/api" }
class GreetingController {
    @GET { path: "/health" }
    def health() Str = "ok"
}

def main() Unit {
    server HttpServer = HttpServer(7070)
    server.addController(GreetingController())
    server.run()
}
```

`HttpServer` itself is written in Lume under
`lume/core/src/main/lume/lume/core/http/HttpServer.lum`.
The only Java HTTP piece is the low-level `JavalinBackend`, which receives
already-discovered route registrations from Lume.

## Run

From this folder:

```bash
gradle build
gradle run
```

Then test:

```bash
curl http://localhost:7070/api/health
curl http://localhost:7070/api/hello
curl -X POST http://localhost:7070/api/echo -d 'from curl'
```

If Gradle is not on `PATH`, pass the executable explicitly:

```bash
GRADLE=/path/to/gradle /path/to/gradle build
```
