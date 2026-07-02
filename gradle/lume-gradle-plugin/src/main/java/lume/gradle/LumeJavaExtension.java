package lume.gradle;

import javax.inject.Inject;

import org.gradle.api.file.ConfigurableFileCollection;
import org.gradle.api.file.DirectoryProperty;
import org.gradle.api.file.RegularFileProperty;
import org.gradle.api.model.ObjectFactory;
import org.gradle.api.provider.Property;

public abstract class LumeJavaExtension {
    private final RegularFileProperty source;
    private final DirectoryProperty generatedSourceDir;
    private final Property<String> mainClass;
    private final Property<String> lumeExecutable;
    private final ConfigurableFileCollection extraClasspath;
    private final ConfigurableFileCollection runtimeClasspath;

    @Inject
    public LumeJavaExtension(ObjectFactory objects) {
        this.source = objects.fileProperty();
        this.generatedSourceDir = objects.directoryProperty();
        this.mainClass = objects.property(String.class).convention("");
        this.lumeExecutable = objects.property(String.class).convention("lume");
        this.extraClasspath = objects.fileCollection();
        this.runtimeClasspath = objects.fileCollection();
    }

    public RegularFileProperty getSource() {
        return source;
    }

    public DirectoryProperty getGeneratedSourceDir() {
        return generatedSourceDir;
    }

    public Property<String> getMainClass() {
        return mainClass;
    }

    public Property<String> getLumeExecutable() {
        return lumeExecutable;
    }

    public ConfigurableFileCollection getExtraClasspath() {
        return extraClasspath;
    }

    public ConfigurableFileCollection getRuntimeClasspath() {
        return runtimeClasspath;
    }
}
