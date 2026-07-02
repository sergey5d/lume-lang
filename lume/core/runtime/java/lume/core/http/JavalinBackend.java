package lume.core.http;

import io.javalin.Javalin;
import lume.runtime.LumeMethod;

public final class JavalinBackend {
    private final int port;
    private final Javalin app;

    public JavalinBackend(long port) {
        this.port = Math.toIntExact(port);
        this.app = Javalin.create();
    }

    public JavalinBackend get(String path, Object controller, LumeMethod method) {
        this.app.get(path, ctx -> ctx.result(String.valueOf(method.invoke(controller))));
        return this;
    }

    public JavalinBackend post(String path, Object controller, LumeMethod method) {
        this.app.post(path, ctx -> ctx.result(String.valueOf(method.invoke(controller, ctx.body()))));
        return this;
    }

    public JavalinBackend run() {
        this.app.start(this.port);
        return this;
    }
}
