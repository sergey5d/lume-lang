package lume.core;

public interface LumeSeekable {
    long position();

    Result<Long, FileError> seek(long offset);

    Result<Long, FileError> seek(long offset, SeekFrom from);
}
