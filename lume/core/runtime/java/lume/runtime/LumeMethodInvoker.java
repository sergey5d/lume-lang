package lume.runtime;

@FunctionalInterface
public interface LumeMethodInvoker {
    Object invoke(Object receiver, Object... args);
}
