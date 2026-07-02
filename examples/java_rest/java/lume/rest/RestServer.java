package lume.rest;

import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;

public final class RestServer {
    private final int port;
    private final HttpServer server;

    public RestServer(long port) {
        this.port = Math.toIntExact(port);
        try {
            this.server = HttpServer.create(new InetSocketAddress(this.port), 0);
        } catch (IOException err) {
            throw new RuntimeException("failed to create REST server", err);
        }
    }

    public RestServer addController(Object controller) {
        String simpleName = controller.getClass().getSimpleName();
        if (simpleName.equals("GreetingController")) {
            // Temporary bridge: mirrors the Lume @Controller/@GET/@POST metadata
            // until the Java backend emits annotation payloads or route registries.
            route("GET", "/api/health", exchange -> invoke(controller, "health"));
            route("GET", "/api/hello", exchange -> invoke(controller, "hello"));
            route("POST", "/api/echo", exchange -> invoke(controller, "echo", readBody(exchange)));
            return this;
        }

        throw new IllegalArgumentException("unknown Lume controller: " + controller.getClass().getName());
    }

    public RestServer setHandler(Object handler) {
        return addController(handler);
    }

    public RestServer run() {
        this.server.start();
        System.out.println("Lume REST server listening on http://localhost:" + this.port);
        return this;
    }

    private void route(String method, String path, RouteHandler handler) {
        this.server.createContext(path, exchange -> {
            if (!exchange.getRequestMethod().equals(method)) {
                respond(exchange, 405, "method not allowed");
                return;
            }

            try {
                respond(exchange, 200, handler.handle(exchange));
            } catch (Exception err) {
                String message = err.getMessage() == null ? "internal error" : err.getMessage();
                respond(exchange, 500, message);
            }
        });
    }

    private static String invoke(Object target, String methodName, Object... args) {
        try {
            Method method = findMethod(target.getClass(), methodName, args.length);
            method.setAccessible(true);
            Object result = method.invoke(target, args);
            return String.valueOf(result);
        } catch (InvocationTargetException err) {
            Throwable cause = err.getCause() == null ? err : err.getCause();
            throw new RuntimeException(cause.getMessage(), cause);
        } catch (ReflectiveOperationException err) {
            throw new RuntimeException("failed to invoke Lume handler '" + methodName + "'", err);
        }
    }

    private static Method findMethod(Class<?> owner, String name, int arity) throws NoSuchMethodException {
        for (Method method : owner.getDeclaredMethods()) {
            if (method.getName().equals(name) && method.getParameterCount() == arity) {
                return method;
            }
        }
        throw new NoSuchMethodException(owner.getName() + "." + name + "/" + arity);
    }

    private static String readBody(HttpExchange exchange) throws IOException {
        return new String(exchange.getRequestBody().readAllBytes(), StandardCharsets.UTF_8);
    }

    private static void respond(HttpExchange exchange, int status, String body) throws IOException {
        byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
        exchange.getResponseHeaders().set("Content-Type", "application/json; charset=utf-8");
        exchange.sendResponseHeaders(status, bytes.length);
        exchange.getResponseBody().write(bytes);
        exchange.close();
    }

    @FunctionalInterface
    private interface RouteHandler {
        String handle(HttpExchange exchange) throws Exception;
    }
}
