package lume.http.javalin.runtime;

import io.javalin.Javalin;

public final class JavalinRuntime {
    private JavalinRuntime() {
    }

    public static void enableVirtualThreads(Javalin app) {
        app.unsafeConfig().useVirtualThreads = true;
    }
}
