# Java REST Bridge Example

This sample keeps the service code clean in Lume and puts HTTP-container glue in Java.

The Lume side declares route metadata:

```txt
@Controller { path: "/api" }
class GreetingController {
    @GET { path: "/health" }
    def health() Str = "ok"
}

def main() Unit {
    server RestServer = RestServer(7070)
    server.setHandler(GreetingController())
    server.run()
}
```

The Java bridge is intentionally boring. It is a tiny library class imported by
the Lume program and backed by a Java HTTP container.

Current bridge:

- `java/lume/rest/RestServer.java`
- uses JDK `com.sun.net.httpserver.HttpServer`
- manually mirrors the Lume route annotations

That last point is temporary. Lume can parse/typecheck the annotations today,
but the Java backend does not yet emit route metadata or generate registration
code from annotation payloads.

## Run

From the repository root:

```bash
mkdir -p /tmp/lume-rest-lib-classes
javac -d /tmp/lume-rest-lib-classes \
  examples/java_rest/java/lume/rest/RestServer.java
jar --create --file /tmp/lume-rest-lib.jar \
  -C /tmp/lume-rest-lib-classes .

cargo run --manifest-path rust/Cargo.toml -p lume -- java \
  examples/java_rest/service.lum \
  --out /tmp/lume-rest \
  --classpath /tmp/lume-rest-lib.jar

javac -cp /tmp/lume-rest-lib.jar -d /tmp/lume-rest-classes \
  java_runtime/src/main/java/lume/runtime/*.java \
  /tmp/lume-rest/examples/java_rest/*.java

java -cp /tmp/lume-rest-classes:/tmp/lume-rest-lib.jar \
  examples.java_rest.Java_restMain
```

Then test:

```bash
curl http://localhost:7070/api/health
curl http://localhost:7070/api/hello
curl -X POST http://localhost:7070/api/echo -d 'from curl'
```

## Javalin Shape

Once the bridge targets Javalin instead of JDK `HttpServer`, the Java glue can
keep the same boundary:

```java
GreetingController controller = new GreetingController();

Javalin app = Javalin.create(config -> {
    config.routes.get("/api/health", ctx -> ctx.result(controller.health()));
    config.routes.get("/api/hello", ctx -> ctx.result(controller.hello()));
    config.routes.post("/api/echo", ctx -> ctx.result(controller.echo(ctx.body())));
}).start(7070);
```

The Lume controller code does not need to change.
