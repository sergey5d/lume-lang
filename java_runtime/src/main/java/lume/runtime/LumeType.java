package lume.runtime;

import java.util.List;

public final class LumeType {
    private final String name;
    private final String qualifiedName;
    private final LumeTypeKind kind;
    private final List<LumeField> fields;
    private final List<LumeMethod> methods;
    private final List<LumeEnumCase> cases;

    private LumeType(
            String name,
            String qualifiedName,
            LumeTypeKind kind,
            List<LumeField> fields,
            List<LumeMethod> methods,
            List<LumeEnumCase> cases) {
        this.name = name;
        this.qualifiedName = qualifiedName;
        this.kind = kind;
        this.fields = List.copyOf(fields);
        this.methods = List.copyOf(methods);
        this.cases = List.copyOf(cases);
    }

    public static LumeType primitive(String name) {
        return new LumeType(name, name, LumeTypeKind.Primitive, List.of(), List.of(), List.of());
    }

    public static LumeType classType(
            String name,
            String qualifiedName,
            LumeField[] fields,
            LumeMethod[] methods) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Class,
                List.of(fields),
                List.of(methods),
                List.of());
    }

    public static LumeType shapeType(
            String name,
            String qualifiedName,
            LumeField[] fields,
            LumeMethod[] methods) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Shape,
                List.of(fields),
                List.of(methods),
                List.of());
    }

    public static LumeType enumType(
            String name,
            String qualifiedName,
            LumeEnumCase[] cases,
            LumeMethod[] methods) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Enum,
                List.of(),
                List.of(methods),
                List.of(cases));
    }

    public static LumeType interfaceType(String name, String qualifiedName, LumeMethod[] methods) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Interface,
                List.of(),
                List.of(methods),
                List.of());
    }

    public static LumeType singleType(
            String name,
            String qualifiedName,
            LumeField[] fields,
            LumeMethod[] methods) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Single,
                List.of(fields),
                List.of(methods),
                List.of());
    }

    public static LumeType annotationType(String name, String qualifiedName, LumeField[] fields) {
        return new LumeType(
                name,
                qualifiedName,
                LumeTypeKind.Annotation,
                List.of(fields),
                List.of(),
                List.of());
    }

    public Option<String> name() {
        return name == null || name.isEmpty() ? Option.none() : Option.some(name);
    }

    public Option<String> qualifiedName() {
        return qualifiedName == null || qualifiedName.isEmpty()
                ? Option.none()
                : Option.some(qualifiedName);
    }

    public LumeTypeKind kind() {
        return kind;
    }

    public Option<LumeType> asClass() {
        return kind == LumeTypeKind.Class ? Option.some(this) : Option.none();
    }

    public Option<LumeType> asShape() {
        return kind == LumeTypeKind.Shape ? Option.some(this) : Option.none();
    }

    public Option<LumeType> asEnum() {
        return kind == LumeTypeKind.Enum ? Option.some(this) : Option.none();
    }

    public Option<LumeType> asInterface() {
        return kind == LumeTypeKind.Interface ? Option.some(this) : Option.none();
    }

    public Option<LumeType> asSingle() {
        return kind == LumeTypeKind.Single ? Option.some(this) : Option.none();
    }

    public Option<LumeType> asAnnotation() {
        return kind == LumeTypeKind.Annotation ? Option.some(this) : Option.none();
    }

    public LumeList<LumeField> fields() {
        return LumeList.from(fields);
    }

    public LumeList<LumeMethod> methods() {
        return LumeList.from(methods);
    }

    public Option<LumeField> field(String name) {
        return fields.stream()
                .filter(field -> field.name().equals(name))
                .findFirst()
                .map(Option::some)
                .orElseGet(Option::none);
    }

    public Option<LumeMethod> method(String name) {
        return methods.stream()
                .filter(method -> method.name().equals(name))
                .findFirst()
                .map(Option::some)
                .orElseGet(Option::none);
    }

    public LumeList<LumeEnumCase> cases() {
        return LumeList.from(cases);
    }

    public Option<LumeEnumCase> case_(String name) {
        return cases.stream()
                .filter(enumCase -> enumCase.name().equals(name))
                .findFirst()
                .map(Option::some)
                .orElseGet(Option::none);
    }

    public LumeType runtimeType() {
        return primitive("Type");
    }

    @Override
    public String toString() {
        return name == null || name.isEmpty() ? kind.toString() : name;
    }
}
