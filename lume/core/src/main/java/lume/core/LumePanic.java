package lume.core;

public final class LumePanic extends RuntimeException {
    public LumePanic(String message) {
        super(message);
    }

    public static <T> T panic(String message) {
        throw new LumePanic(message);
    }
}
