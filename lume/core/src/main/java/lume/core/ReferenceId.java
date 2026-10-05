package lume.core;

/** Opaque, strongly held identity token for a Lume reference-bearing value. */
public final class ReferenceId implements Hashed<ReferenceId>, LumeTyped {
    private static final LumeType TYPE = LumeType.classType(
            "ReferenceId",
            "lume.core.ReferenceId",
            new LumeField[] {},
            new LumeMethod[] {});

    private final Object reference;

    ReferenceId(Object reference) {
        this.reference = reference;
    }

    @Override
    public boolean equals(Object other) {
        return other instanceof ReferenceId id && reference == id.reference;
    }

    public Boolean equals(ReferenceId other) {
        return reference == other.reference;
    }

    @Override
    public int hashCode() {
        return System.identityHashCode(reference);
    }

    @Override
    public Long hash() {
        return (long) hashCode();
    }

    @Override
    public LumeType runtimeType() {
        return TYPE;
    }

    @Override
    public String toString() {
        return "<reference>";
    }
}
