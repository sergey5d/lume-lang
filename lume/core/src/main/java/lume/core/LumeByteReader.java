package lume.core;

public interface LumeByteReader {
    Result<LumeVector<Long>, FileError> read(long maxBytes);

    Result<LumeVector<Long>, FileError> readToEnd();
}
