package lume.gradle;

import org.gradle.api.Plugin;
import org.gradle.api.Project;
import org.gradle.api.file.Directory;
import org.gradle.api.plugins.JavaApplication;
import org.gradle.api.plugins.JavaPluginExtension;
import org.gradle.api.tasks.Exec;
import org.gradle.api.tasks.SourceSet;
import org.gradle.api.tasks.SourceSetContainer;
import org.gradle.api.tasks.bundling.Jar;

public final class LumeJavaApplicationPlugin implements Plugin<Project> {
    @Override
    public void apply(Project project) {
        project.getPluginManager().apply("application");
        project.getPluginManager().apply("java");

        LumeJavaExtension extension = project.getExtensions().create("lumeJava", LumeJavaExtension.class);
        extension.getSource().convention(project.getLayout().getProjectDirectory().file("src/main/lume/service.lum"));
        extension.getGeneratedSourceDir().convention(project.getLayout().getBuildDirectory().dir("generated/sources/lume/java"));

        SourceSetContainer sourceSets = project.getExtensions()
            .getByType(JavaPluginExtension.class)
            .getSourceSets();
        sourceSets.named(SourceSet.MAIN_SOURCE_SET_NAME, sourceSet ->
            sourceSet.getJava().srcDir(extension.getGeneratedSourceDir())
        );

        project.getExtensions().configure(JavaApplication.class, application ->
            application.getMainClass().set(extension.getMainClass())
        );

        project.getDependencies().add("implementation", extension.getRuntimeClasspath());

        var generateJava = project.getTasks().register("generateLumeJava", Exec.class, task -> {
            task.setDescription("Generates Java sources from Lume sources.");
            task.setGroup("build");
            task.getInputs().file(extension.getSource());
            task.getInputs().files(extension.getExtraClasspath());
            task.getInputs().property("lumeExecutable", extension.getLumeExecutable());
            task.getOutputs().dir(extension.getGeneratedSourceDir());

            task.doFirst(action -> {
                Directory outputDir = extension.getGeneratedSourceDir().get();
                project.delete(outputDir);
                outputDir.getAsFile().mkdirs();

                task.setCommandLine(
                    extension.getLumeExecutable().get(),
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
            task.from(project.provider(() ->
                project.getConfigurations().getByName("runtimeClasspath").getFiles().stream()
                    .map(file -> file.isDirectory() ? file : project.zipTree(file))
                    .toList()
            ));
        });
    }
}
