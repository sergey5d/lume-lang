package lume.core;

public interface LumeCloseable {
    Boolean closed();

    Result<LumeUnit, FileError> close();
}
