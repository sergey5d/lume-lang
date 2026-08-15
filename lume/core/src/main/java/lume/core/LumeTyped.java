package lume.core;

/** Supplies a Lume runtime type witness without reflective member lookup. */
public interface LumeTyped {
    LumeType runtimeType();
}
