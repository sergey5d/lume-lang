package lume.core;

public interface LumeTextReader extends LumeCloseable {
    Result<Option<String>, FileError> readLine();

    Result<String, FileError> readToEnd();
}
