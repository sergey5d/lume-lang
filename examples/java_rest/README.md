# Java REST Example

This is the plain source version of the REST example. The Gradle sample is the
preferred runnable form because it builds `lume-core.jar` automatically.

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

From the repository root:

```bash
/tmp/gradle-9.6.1/bin/gradle -p lume/core jar --no-daemon

cargo run --manifest-path rust/Cargo.toml -p lume -- java \
  examples/java_rest/service.lum \
  --out /tmp/lume-rest \
  --classpath lume/core/build/libs/lume-core.jar

javac -cp lume/core/build/libs/lume-core.jar -d /tmp/lume-rest-classes \
  /tmp/lume-rest/examples/java_rest/*.java

java -cp /tmp/lume-rest-classes:lume/core/build/libs/lume-core.jar \
  examples.java_rest.Java_restMain
```

Then test:

```bash
curl http://localhost:7070/api/health
curl http://localhost:7070/api/hello
curl -X POST http://localhost:7070/api/echo -d 'from curl'
```

For day-to-day use, prefer `examples/java_gradle_rest`, which wraps these steps
in Gradle.
