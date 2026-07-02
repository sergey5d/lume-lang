package lume.gradle;

import java.io.File;
import java.util.Locale;

import org.gradle.api.Plugin;
import org.gradle.api.Project;
import org.gradle.api.file.Directory;
import org.gradle.api.file.ProjectLayout;
import org.gradle.api.plugins.JavaApplication;
import org.gradle.api.plugins.JavaPluginExtension;
import org.gradle.api.provider.Provider;
import org.gradle.api.tasks.Exec;
import org.gradle.api.tasks.SourceSet;
import org.gradle.api.tasks.SourceSetContainer;
import org.gradle.api.tasks.bundling.Jar;

public final class LumeJavaApplicationPlugin implements Plugin<Project> {
    @Override
    public void apply(Project project) {
        project.getPluginManager().apply("application");
        project.getPluginManager().apply("java");

        File repoRoot = findRepoRoot(project);
        ProjectLayout layout = project.getLayout();

        LumeJavaExtension extension = project.getExtensions().create("lumeJava", LumeJavaExtension.class);
        extension.getSource().convention(layout.getProjectDirectory().file("src/main/lume/service.lum"));
        extension.getGeneratedSourceDir().convention(layout.getBuildDirectory().dir("generated/sources/lume/java"));

        File rustManifest = new File(repoRoot, "rust/Cargo.toml");
        File compilerSources = new File(repoRoot, "rust/crates/lume/src");
        File compilerBinary = new File(repoRoot, compilerBinaryPath());
        File coreProjectDir = new File(repoRoot, "lume/core");
        File coreJar = new File(coreProjectDir, "build/libs/lume-core.jar");

        project.getDependencies().add("implementation", project.files(coreJar));

        SourceSetContainer sourceSets = project.getExtensions()
            .getByType(JavaPluginExtension.class)
            .getSourceSets();
        sourceSets.named(SourceSet.MAIN_SOURCE_SET_NAME, sourceSet ->
            sourceSet.getJava().srcDir(extension.getGeneratedSourceDir())
        );

        project.getExtensions().configure(JavaApplication.class, application ->
            application.getMainClass().set(extension.getMainClass())
        );

        var buildCompiler = project.getTasks().register("buildLumeCompiler", Exec.class, task -> {
            task.setDescription("Builds the repo-local Lume compiler unless LUME points to an installed compiler.");
            task.setGroup("build");
            task.onlyIf(spec -> System.getenv("LUME") == null || System.getenv("LUME").isBlank());
            task.getInputs().file(rustManifest);
            task.getInputs().files(project.fileTree(compilerSources));
            task.getOutputs().file(compilerBinary);
            task.commandLine("cargo", "build", "--manifest-path", rustManifest.getAbsolutePath(), "-p", "lume");
        });

        String gradleExecutable = envOrDefault("GRADLE", "gradle");
        var buildCore = project.getTasks().register("buildLumeCore", Exec.class, task -> {
            task.setDescription("Builds lume-core.jar for app generation and compilation.");
            task.setGroup("build");
            task.dependsOn(buildCompiler);
            task.getInputs().files(project.fileTree(coreProjectDir));
            task.getInputs().file(rustManifest);
            task.getInputs().files(project.fileTree(compilerSources));
            task.getOutputs().file(coreJar);
            task.commandLine(
                gradleExecutable,
                "-p",
                coreProjectDir.getAbsolutePath(),
                "jar",
                "--no-daemon"
            );
        });

        var generateJava = project.getTasks().register("generateLumeJava", Exec.class, task -> {
            task.setDescription("Generates Java sources from Lume sources.");
            task.setGroup("build");
            task.dependsOn(buildCore);
            task.getInputs().file(extension.getSource());
            task.getInputs().file(coreJar);
            task.getInputs().files(project.fileTree(compilerSources));
            task.getInputs().property("lumeExecutable", lumeExecutable(compilerBinary));
            task.getInputs().files(extension.getExtraClasspath());
            task.getOutputs().dir(extension.getGeneratedSourceDir());

            task.doFirst(action -> {
                Directory outputDir = extension.getGeneratedSourceDir().get();
                project.delete(outputDir);
                outputDir.getAsFile().mkdirs();

                task.setCommandLine(
                    lumeExecutable(compilerBinary),
                    "gen",
                    extension.getSource().get().getAsFile().getAbsolutePath(),
                    "--out",
                    outputDir.getAsFile().getAbsolutePath()
                );
                if (!extension.getExtraClasspath().isEmpty()) {
                    task.args("--classpath", extension.getExtraClasspath().getAsPath());
                }
            });
        });

        project.getTasks().named("compileJava").configure(task -> task.dependsOn(generateJava));

        project.getTasks().named("jar", Jar.class).configure(task -> {
            task.setDuplicatesStrategy(org.gradle.api.file.DuplicatesStrategy.EXCLUDE);
            task.doFirst(action ->
                task.getManifest().attributes(java.util.Map.of("Main-Class", extension.getMainClass().get()))
            );
            Provider<java.util.List<Object>> runtimeClasspathEntries = project.provider(() ->
                project.getConfigurations().getByName("runtimeClasspath").getFiles().stream()
                .map(file -> file.isDirectory() ? file : project.zipTree(file))
                .toList()
            );
            task.from(runtimeClasspathEntries);
        });
    }

    private static String envOrDefault(String name, String fallback) {
        String value = System.getenv(name);
        return value == null || value.isBlank() ? fallback : value;
    }

    private static String lumeExecutable(File compilerBinary) {
        return envOrDefault("LUME", compilerBinary.getAbsolutePath());
    }

    private static String compilerBinaryPath() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        return os.contains("win") ? "rust/target/debug/lume.exe" : "rust/target/debug/lume";
    }

    private static File findRepoRoot(Project project) {
        File current = project.getProjectDir();
        while (current != null) {
            if (new File(current, "rust/Cargo.toml").isFile()
                && new File(current, "lume/core/build.gradle.kts").isFile()) {
                return current;
            }
            current = current.getParentFile();
        }
        throw new IllegalStateException("could not find repository root from " + project.getProjectDir());
    }
}
