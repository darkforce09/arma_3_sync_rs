package harness;

import java.io.IOException;
import java.io.InputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.stream.Stream;

/**
 * Entry point of the Java fixture harness (see README.md).
 *
 * <p>stdout is the protocol stream: Arma3Sync prints progress with {@code System.out}, so
 * {@code System.out} is redirected to stderr at startup and only protocol lines and JSON reach the
 * saved stream. Exit codes: 0 ok, 1 verification failure, 2 usage, 3 internal error.
 */
public final class Main {

	static final int EXIT_OK = 0;
	static final int EXIT_FAILED = 1;
	static final int EXIT_USAGE = 2;
	static final int EXIT_INTERNAL = 3;

	static final List<String> PARTS = List.of("descriptors", "local", "built", "presets", "roundtrip");
	static final long MAX_TOTAL_BYTES = 3L * 1024 * 1024;

	private static PrintStream protocol;

	private Main() {
	}

	public static void main(String[] args) {
		protocol = new PrintStream(System.out, false, StandardCharsets.UTF_8);
		System.setOut(System.err);
		int code;
		try {
			code = run(args);
		} catch (UsageException e) {
			System.err.println("java_fixture_harness: " + e.getMessage());
			code = EXIT_USAGE;
		} catch (Throwable t) {
			t.printStackTrace();
			code = EXIT_INTERNAL;
		}
		protocol.flush();
		System.err.flush();
		// Arma3Sync executors may leave non-daemon threads behind.
		System.exit(code);
	}

	/** Writes protocol text (no implicit newline). */
	static void emit(String text) {
		protocol.print(text);
	}

	static Path harnessHome() {
		String home = System.getProperty("a3s.harness.home");
		if (home == null || home.isBlank()) {
			throw new IllegalStateException("a3s.harness.home is not set (start the harness with run.sh)");
		}
		return Paths.get(home);
	}

	static String javaCommit() {
		return System.getProperty("a3s.harness.javaCommit", "unknown");
	}

	private static int run(String[] args) throws Exception {
		if (args.length == 0) {
			throw new UsageException(
					"usage: run.sh <descriptors|generate|verify|dump|roundtrip|selftest> [args]");
		}
		String command = args[0];
		List<String> rest = new ArrayList<String>(Arrays.asList(args).subList(1, args.length));
		switch (command) {
		case "descriptors":
			noArgs(rest);
			emit(Descriptors.render());
			return EXIT_OK;
		case "generate":
			return generate(rest);
		case "verify":
			return Verify.verifyCommand(rest);
		case "dump":
			return Verify.dumpCommand(rest);
		case "roundtrip":
			return Verify.roundtripCommand(rest);
		case "selftest":
			noArgs(rest);
			return Verify.selftest();
		case "child-local":
			LocalFixtures.child(rest);
			return EXIT_OK;
		case "child-variant":
			LocalFixtures.childVariant(rest);
			return EXIT_OK;
		case "child-built":
			BuiltFixtures.child(rest);
			return EXIT_OK;
		case "child-events":
			EventsSession.child(rest);
			return EXIT_OK;
		default:
			throw new UsageException("unknown command: " + command);
		}
	}

	private static void noArgs(List<String> rest) throws UsageException {
		if (!rest.isEmpty()) {
			throw new UsageException("unexpected arguments: " + rest);
		}
	}

	private static int generate(List<String> args) throws Exception {
		Path out = null;
		Path work = null;
		boolean keepWork = false;
		Set<String> parts = new LinkedHashSet<String>(PARTS);
		for (int i = 0; i < args.size(); i++) {
			String arg = args.get(i);
			switch (arg) {
			case "--out":
				out = Paths.get(value(args, ++i, arg)).toAbsolutePath().normalize();
				break;
			case "--work":
				work = Paths.get(value(args, ++i, arg)).toAbsolutePath().normalize();
				break;
			case "--keep-work":
				keepWork = true;
				break;
			case "--only":
				parts.clear();
				for (String part : value(args, ++i, arg).split(",")) {
					if (!PARTS.contains(part)) {
						throw new UsageException("unknown part: " + part + " (parts: " + String.join(",", PARTS) + ")");
					}
					parts.add(part);
				}
				break;
			default:
				throw new UsageException("unknown generate argument: " + arg);
			}
		}
		if (out == null) {
			throw new UsageException("generate requires --out <dir>");
		}
		if (work != null && (work.startsWith(out) || out.startsWith(work))) {
			throw new UsageException("--work and --out must not contain each other: " + work + ", " + out);
		}
		requireEmptyOrMissing(out, "--out");
		Files.createDirectories(out);
		if (work == null) {
			work = Files.createTempDirectory("a3s-harness").toAbsolutePath().normalize();
			if (work.startsWith(out)) {
				Files.delete(work);
				throw new UsageException("the temporary work directory " + work + " is inside --out " + out);
			}
		} else {
			requireEmptyOrMissing(work, "--work");
			Files.createDirectories(work);
		}
		System.err.println("generate: out " + out + ", work " + work + ", parts " + parts);
		try {
			if (parts.contains("descriptors")) {
				Io.writeString(out.resolve("java_descriptors.txt"), Descriptors.render());
			}
			if (parts.contains("local")) {
				LocalFixtures.generate(out, work.resolve("local"));
				Verify.selfCheck(out.resolve("java_local"));
			}
			if (parts.contains("built")) {
				BuiltFixtures.generate(out, work.resolve("built"));
				Verify.selfCheck(out.resolve("java_built"));
			}
			if (parts.contains("presets")) {
				PresetFixtures.generate(out, work.resolve("presets"));
			}
			if (parts.contains("roundtrip")) {
				RoundtripFixtures.generate(out);
			}
			checkNoWorkPath(out, work);
			Manifest.write(out);
		} finally {
			if (!keepWork) {
				Io.deleteTree(work);
			}
		}
		return EXIT_OK;
	}

	static String value(List<String> args, int index, String option) throws UsageException {
		if (index >= args.size()) {
			throw new UsageException(option + " requires a value");
		}
		return args.get(index);
	}

	private static void requireEmptyOrMissing(Path dir, String option) throws IOException, UsageException {
		if (!Files.exists(dir)) {
			return;
		}
		if (!Files.isDirectory(dir)) {
			throw new UsageException(option + " " + dir + " is not a directory");
		}
		try (Stream<Path> children = Files.list(dir)) {
			if (children.findAny().isPresent()) {
				throw new UsageException(option + " " + dir + " must be empty or missing");
			}
		}
	}

	/** B13: no generated file may contain the absolute work path (gzip files are checked inflated). */
	private static void checkNoWorkPath(Path out, Path work) throws IOException, HarnessException {
		Set<String> needles = new LinkedHashSet<String>();
		needles.add(work.toString());
		if (Files.exists(work)) {
			needles.add(work.toRealPath().toString());
		}
		for (String rel : Io.files(out)) {
			byte[] bytes = Files.readAllBytes(out.resolve(rel));
			if (Io.isGzip(bytes)) {
				try {
					bytes = Io.gunzip(bytes);
				} catch (IOException e) {
					// Not a gzip stream after all; search the raw bytes.
				}
			}
			String text = new String(bytes, StandardCharsets.ISO_8859_1);
			for (String needle : needles) {
				String encoded = new String(needle.getBytes(StandardCharsets.UTF_8), StandardCharsets.ISO_8859_1);
				if (text.contains(encoded)) {
					throw new HarnessException("generated file " + rel + " contains the work path " + needle);
				}
			}
		}
	}

	/**
	 * B12: runs {@code harness.Main <args>} in a child JVM with fresh statics. Child stdout and
	 * stderr both go to this process's stderr.
	 */
	static void runChild(String variant, Map<String, String> properties, String... args)
			throws IOException, HarnessException, InterruptedException {
		String javaExecutable = ProcessHandle.current().info().command()
				.orElseThrow(() -> new HarnessException("cannot determine the java executable"));
		String classpath = System.getProperty("java.class.path");
		if (variant != null) {
			classpath = harnessHome().resolve("build").resolve("variants").resolve(variant) + java.io.File.pathSeparator
					+ classpath;
		}
		List<String> command = new ArrayList<String>();
		command.add(javaExecutable);
		for (String key : List.of("user.timezone", "user.language", "user.country", "file.encoding",
				"sun.jnu.encoding", "java.awt.headless", "java.io.tmpdir", "a3s.harness.home",
				"a3s.harness.javaCommit")) {
			String value = System.getProperty(key);
			if (value != null) {
				command.add("-D" + key + "=" + value);
			}
		}
		for (Map.Entry<String, String> property : properties.entrySet()) {
			command.add("-D" + property.getKey() + "=" + property.getValue());
		}
		command.add("-cp");
		command.add(classpath);
		command.add(Main.class.getName());
		command.addAll(Arrays.asList(args));
		System.err.println("child: " + String.join(" ", args));
		ProcessBuilder builder = new ProcessBuilder(command).redirectErrorStream(true);
		Process process = builder.start();
		try (InputStream in = process.getInputStream()) {
			in.transferTo(System.err);
		}
		int code = process.waitFor();
		if (code != 0) {
			throw new HarnessException("child JVM `" + String.join(" ", args) + "` exited with code " + code);
		}
	}

	/** The B12 per-user properties for a child whose state lives below {@code work}. */
	static Map<String, String> perUserProperties(Path work) throws IOException {
		Files.createDirectories(work.resolve("home"));
		Map<String, String> properties = new java.util.LinkedHashMap<String, String>();
		properties.put("user.home", work.resolve("home").toString());
		properties.put("a3s.configPath", work.resolve("config").toString());
		properties.put("a3s.dataPath", work.resolve("data").toString());
		properties.put("a3s.cachePath", work.resolve("cache").toString());
		return properties;
	}
}
