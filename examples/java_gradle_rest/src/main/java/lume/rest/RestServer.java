package lume.rest;

import io.javalin.Javalin;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;

public final class RestServer {
    private final int port;
    private final Javalin app;

    public RestServer(long port) {
        this.port = Math.toIntExact(port);
        this.app = Javalin.create();
    }

    public RestServer setHandler(Object handler) {
        return addController(handler);
    }

    public RestServer addController(Object controller) {
        String simpleName = controller.getClass().getSimpleName();
        if (simpleName.equals("GreetingController")) {
            // Temporary bridge: mirrors the Lume @Controller/@GET/@POST metadata
            // until the backend emits annotation payloads or route registries.
            this.app.get("/api/health", ctx -> ctx.result(invoke(controller, "health")));
            this.app.get("/api/hello", ctx -> ctx.result(invoke(controller, "hello")));
            this.app.post("/api/echo", ctx -> ctx.result(invoke(controller, "echo", ctx.body())));
            return this;
        }

        throw new IllegalArgumentException("unknown Lume controller: " + controller.getClass().getName());
    }

    public RestServer run() {
        this.app.start(this.port);
        return this;
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
}
