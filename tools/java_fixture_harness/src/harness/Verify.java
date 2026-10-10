package harness;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedList;
import java.util.List;
import java.util.Map;

import fr.soe.a3s.constant.CheckRepositoriesFrequency;
import fr.soe.a3s.constant.IconResize;
import fr.soe.a3s.constant.LookAndFeel;
import fr.soe.a3s.constant.MinimizationType;
import fr.soe.a3s.constant.StartWithOS;
import fr.soe.a3s.dao.A3SFilesAccessor;
import fr.soe.a3s.dao.CommonDAO;
import fr.soe.a3s.dao.EncryptionProvider;
import fr.soe.a3s.dao.FileAccessMethods;
import fr.soe.a3s.dao.zip.ZipDAO;
import fr.soe.a3s.domain.Preferences;
import fr.soe.a3s.jazsyncmake.MetaFileMaker;

/**
 * {@code verify}, {@code dump}, {@code roundtrip} and {@code selftest}: Arma3Sync's own readers
 * applied to files produced elsewhere (typically by the Rust implementation).
 */
public final class Verify {

	private static final Map<String, String> A3S_FILES = Map.of("sync",
			"fr.soe.a3s.domain.repository.SyncTreeDirectory", "serverinfo", "fr.soe.a3s.domain.repository.ServerInfo",
			"changelogs", "fr.soe.a3s.domain.repository.Changelogs", "autoconfig",
			"fr.soe.a3s.domain.repository.AutoConfig", "events", "fr.soe.a3s.domain.repository.Events");

	enum Reader {
		/** {@code A3SFilesAccessor.read(File)}. */
		PLAIN,
		/** {@code A3SFilesAccessor.read(Cipher, File)} with the repository cipher. */
		ENCRYPTED,
		/** {@code CommonDAO.importAutoConfig(File)}. */
		AUTOCONFIG,
		ZIP, ZSYNC, SKIP
	}

	/** How a file is read and which root class it must have. */
	static final class Kind {
		final Reader reader;
		final String expected;

		Kind(Reader reader, String expected) {
			this.reader = reader;
			this.expected = expected;
		}

		boolean isObject() {
			return reader == Reader.PLAIN || reader == Reader.ENCRYPTED || reader == Reader.AUTOCONFIG;
		}
	}

	private Verify() {
	}

	static Kind classify(Path file) {
		String name = file.getFileName().toString();
		Path parent = file.toAbsolutePath().getParent();
		if (parent != null && parent.getFileName() != null && parent.getFileName().toString().equals(".a3s")
				&& A3S_FILES.containsKey(name)) {
			return new Kind(Reader.PLAIN, A3S_FILES.get(name));
		}
		if (name.endsWith(".a3s.repository")) {
			return new Kind(Reader.ENCRYPTED, "fr.soe.a3s.domain.repository.Repository");
		}
		if (name.endsWith(".repository.gz")) {
			return new Kind(Reader.PLAIN, "fr.soe.a3s.domain.repository.Repository");
		}
		if (name.endsWith(".a3s.profile")) {
			return new Kind(Reader.PLAIN, "fr.soe.a3s.domain.Profile");
		}
		if (name.equals("a3s.cfg") || name.endsWith(".a3s.cfg")) {
			return new Kind(Reader.PLAIN, "fr.soe.a3s.domain.configration.Configuration");
		}
		if (name.equals("a3s.prefs") || name.endsWith(".a3s.prefs")) {
			return new Kind(Reader.PLAIN, "fr.soe.a3s.domain.Preferences");
		}
		if (name.endsWith(".a3s.autoconfig")) {
			return new Kind(Reader.AUTOCONFIG, "fr.soe.a3s.domain.AutoConfig");
		}
		if (name.endsWith(".pbo.zip") || name.endsWith(".ebo.zip")) {
			return new Kind(Reader.ZIP, null);
		}
		if (name.endsWith(".zsync")) {
			return new Kind(Reader.ZSYNC, null);
		}
		return new Kind(Reader.SKIP, null);
	}

	/** Reads an object file with the reader Arma3Sync uses for it. */
	static Object readObject(Path file, Kind kind) throws Exception {
		switch (kind.reader) {
		case PLAIN:
			// Mirrors dao/A3SFilesAccessor.java:55-83
			return A3SFilesAccessor.read(file.toFile());
		case ENCRYPTED:
			// Mirrors dao/A3SFilesAccessor.java:85-114 with dao/EncryptionProvider.java:27-33
			return A3SFilesAccessor.read(EncryptionProvider.getDecryptionCipher(), file.toFile());
		case AUTOCONFIG:
			// Mirrors dao/CommonDAO.java:22-34
			return new CommonDAO().importAutoConfig(file.toFile());
		default:
			throw new HarnessException(file + " is not an object file");
		}
	}

	/** Reads an object file and checks its root class. */
	static Object readChecked(Path file) throws Exception {
		Kind kind = classify(file);
		Object object = readObject(file, kind);
		checkRoot(object, kind);
		return object;
	}

	private static void checkRoot(Object object, Kind kind) throws HarnessException {
		String actual = object == null ? "null" : object.getClass().getName();
		if (!actual.equals(kind.expected)) {
			throw new HarnessException("expected " + kind.expected + ", got " + actual);
		}
	}

	static int verifyCommand(List<String> args) throws Exception {
		Path dumpDir = null;
		boolean semantic = false;
		List<Path> paths = new ArrayList<Path>();
		for (int i = 0; i < args.size(); i++) {
			String arg = args.get(i);
			if (arg.equals("--dump-dir")) {
				dumpDir = Paths.get(Main.value(args, ++i, arg));
			} else if (arg.equals("--semantic")) {
				semantic = true;
			} else if (arg.startsWith("--")) {
				throw new UsageException("unknown verify option: " + arg);
			} else {
				paths.add(Paths.get(arg));
			}
		}
		if (paths.isEmpty()) {
			throw new UsageException("usage: verify [--dump-dir <dir>] [--semantic] <path>...");
		}
		int ok = 0;
		int fail = 0;
		int skip = 0;
		for (Path root : paths) {
			List<String> rels;
			if (Files.isDirectory(root)) {
				rels = Io.files(root);
			} else if (Files.isRegularFile(root)) {
				rels = List.of("");
			} else {
				Main.emit("FAIL\t" + root + "\tpath does not exist\n");
				fail++;
				continue;
			}
			for (String rel : rels) {
				Path file = rel.isEmpty() ? root : root.resolve(rel);
				String dumpRel = rel.isEmpty() ? root.getFileName().toString() : rel;
				Kind kind = classify(file);
				if (kind.reader == Reader.SKIP) {
					Main.emit("SKIP\t" + file + "\n");
					skip++;
					continue;
				}
				try {
					String detail = verifyFile(file, kind, dumpDir == null ? null : dumpDir.resolve(dumpRel + ".json"),
							semantic);
					Main.emit("OK\t" + file + "\t" + oneLine(detail) + "\n");
					ok++;
				} catch (Throwable t) {
					Main.emit("FAIL\t" + file + "\t" + describe(t) + "\n");
					fail++;
				}
			}
		}
		Main.emit("SUMMARY\tok=" + ok + "\tfail=" + fail + "\tskip=" + skip + "\n");
		return fail > 0 ? Main.EXIT_FAILED : Main.EXIT_OK;
	}

	/**
	 * Generation self-check: every classified file below {@code root} must pass. Background
	 * failures inside Arma3Sync's executors (for example a missing digest class) are swallowed by
	 * {@code invokeAll}; re-verifying the output catches them.
	 */
	static void selfCheck(Path root) throws Exception {
		List<String> failures = new ArrayList<String>();
		int checked = 0;
		for (String rel : Io.files(root)) {
			Path file = root.resolve(rel);
			Kind kind = classify(file);
			if (kind.reader == Reader.SKIP) {
				continue;
			}
			checked++;
			try {
				verifyFile(file, kind, null, false);
			} catch (Throwable t) {
				failures.add(rel + ": " + describe(t));
			}
		}
		if (!failures.isEmpty()) {
			throw new HarnessException("self-check of " + root.getFileName() + " failed:\n" + String.join("\n", failures));
		}
		System.err.println("self-check: " + checked + " files below " + root.getFileName() + " verified");
	}

	private static String verifyFile(Path file, Kind kind, Path dump, boolean semantic) throws Exception {
		switch (kind.reader) {
		case ZIP:
			return verifyZip(file);
		case ZSYNC:
			return verifyZsync(file);
		default:
			Object object = readObject(file, kind);
			checkRoot(object, kind);
			if (dump != null) {
				Io.writeString(dump, JsonDump.dump(object, semantic));
			}
			return object.getClass().getName();
		}
	}

	/** Unzips a copy with {@code ZipDAO.unZip} (which deletes its input) and compares SHA-1s. */
	private static String verifyZip(Path zip) throws Exception {
		Path tmp = Files.createTempDirectory("a3s-verify-zip");
		try {
			Path copy = tmp.resolve(zip.getFileName().toString());
			Files.copy(zip, copy, StandardCopyOption.REPLACE_EXISTING);
			// Mirrors dao/zip/ZipDAO.java:29-102
			new ZipDAO().unZip(copy.toFile());
			String name = zip.getFileName().toString();
			Path unzipped = tmp.resolve(name.substring(0, name.length() - ".zip".length()));
			if (!Files.isRegularFile(unzipped)) {
				throw new HarnessException("unZip produced no file");
			}
			String entrySha1 = FileAccessMethods.computeSHA1(unzipped.toFile());
			Path source = zip.resolveSibling(unzipped.getFileName().toString());
			if (!Files.isRegularFile(source)) {
				return "unzipped sha1 " + entrySha1 + " (no source file)";
			}
			String sourceSha1 = FileAccessMethods.computeSHA1(source.toFile());
			if (!entrySha1.equals(sourceSha1)) {
				throw new HarnessException("sha1 mismatch: zip entry " + entrySha1 + ", source " + sourceSha1);
			}
			return "sha1 " + entrySha1;
		} finally {
			Io.deleteTree(tmp);
		}
	}

	/** Regenerates the {@code .zsync} with jazsyncmake and compares whole files. */
	private static String verifyZsync(Path zsync) throws Exception {
		String name = zsync.getFileName().toString();
		Path source = zsync.resolveSibling(name.substring(0, name.length() - ".zsync".length()));
		if (!Files.isRegularFile(source)) {
			throw new HarnessException("source file missing: " + source.getFileName());
		}
		byte[] actual = Files.readAllBytes(zsync);
		String url = null;
		for (String line : new String(actual, StandardCharsets.ISO_8859_1).split("\n", -1)) {
			if (line.isEmpty()) {
				break;
			}
			if (line.startsWith("URL: ")) {
				url = new String(line.substring(5).getBytes(StandardCharsets.ISO_8859_1), StandardCharsets.UTF_8);
			}
		}
		if (url == null) {
			throw new HarnessException("no URL header line");
		}
		Path tmp = Files.createTempFile("a3s-verify", ".zsync");
		try {
			// Mirrors dao/repository/RepositoryZsyncProcessor.java:64-83
			new MetaFileMaker(source.toFile(), tmp.toFile(), url, FileAccessMethods.computeSHA1(source.toFile()));
			byte[] expected = Files.readAllBytes(tmp);
			int mismatch = Arrays.mismatch(expected, actual);
			if (mismatch >= 0) {
				throw new HarnessException("zsync differs from the Java regeneration at byte " + mismatch + " (java "
						+ expected.length + " bytes, file " + actual.length + " bytes)");
			}
			return "zsync identical (" + actual.length + " bytes)";
		} finally {
			Files.deleteIfExists(tmp);
		}
	}

	static int dumpCommand(List<String> args) throws Exception {
		boolean semantic = false;
		Path file = null;
		for (String arg : args) {
			if (arg.equals("--semantic")) {
				semantic = true;
			} else if (arg.startsWith("--") || file != null) {
				throw new UsageException("usage: dump [--semantic] <file>");
			} else {
				file = Paths.get(arg);
			}
		}
		if (file == null) {
			throw new UsageException("usage: dump [--semantic] <file>");
		}
		Kind kind = classify(file);
		if (!kind.isObject()) {
			throw new HarnessException(file + " is not an object file");
		}
		Object object = readObject(file, kind);
		checkRoot(object, kind);
		Main.emit(JsonDump.dump(object, semantic));
		return Main.EXIT_OK;
	}

	static int roundtripCommand(List<String> args) throws Exception {
		if (args.size() != 2) {
			throw new UsageException("usage: roundtrip <in> <out>");
		}
		Path in = Paths.get(args.get(0));
		Path out = Paths.get(args.get(1));
		Object object = A3SFilesAccessor.read(in.toFile());
		Io.writeUploadStyle(object, out);
		Main.emit("OK\t" + out + "\t" + object.getClass().getName() + "\n");
		return Main.EXIT_OK;
	}

	/** Ported from {@code src/test/java/fr/soe/a3s/dao/A3SFilesAccessorTest.java}. */
	static int selftest() throws Exception {
		Path tmp = Files.createTempDirectory("a3s-selftest");
		int ok = 0;
		int fail = 0;
		try {
			String[] names = { "preferences_roundtrip", "rejects_class_outside_allowlist", "accepts_object_array" };
			for (int i = 0; i < names.length; i++) {
				try {
					String detail = selftestCheck(i, tmp.toFile());
					Main.emit("OK\tselftest/" + names[i] + "\t" + detail + "\n");
					ok++;
				} catch (Throwable t) {
					Main.emit("FAIL\tselftest/" + names[i] + "\t" + describe(t) + "\n");
					fail++;
				}
			}
		} finally {
			Io.deleteTree(tmp);
		}
		Main.emit("SUMMARY\tok=" + ok + "\tfail=" + fail + "\tskip=0\n");
		return fail > 0 ? Main.EXIT_FAILED : Main.EXIT_OK;
	}

	private static String selftestCheck(int index, File dir) throws Exception {
		switch (index) {
		case 0: {
			// A3SFilesAccessorTest.java:27-46
			Preferences preferences = new Preferences();
			preferences.setLaunchPanelGameLaunch(MinimizationType.TRAY);
			preferences.setLaunchPanelMinimized(MinimizationType.TASK_BAR);
			preferences.setLookAndFeel(LookAndFeel.LAF_GRAPHITE);
			preferences.setIconResizeSize(IconResize.AUTO);
			preferences.setStartWithOS(StartWithOS.ENABLED);
			preferences.setCheckRepositoriesFrequency(CheckRepositoriesFrequency.FREQ1);
			File file = new File(dir, "prefs.bin");
			A3SFilesAccessor.write(preferences, file);
			Preferences restored = (Preferences) A3SFilesAccessor.read(file);
			if (restored.getLookAndFeel() != preferences.getLookAndFeel()
					|| restored.getLaunchPanelGameLaunch() != preferences.getLaunchPanelGameLaunch()
					|| restored.getStartWithOS() != preferences.getStartWithOS()
					|| restored.getCheckRepositoriesFrequency() != preferences.getCheckRepositoriesFrequency()) {
				throw new HarnessException("enum values differ after the round trip");
			}
			return "4 enum values kept";
		}
		case 1: {
			// A3SFilesAccessorTest.java:50-54
			File file = new File(dir, "unexpected.bin");
			A3SFilesAccessor.write(new LinkedList<String>(), file);
			try {
				A3SFilesAccessor.read(file);
			} catch (IOException e) {
				return "rejected: " + oneLine(e.getMessage());
			}
			throw new HarnessException("java.util.LinkedList was accepted");
		}
		default: {
			// A3SFilesAccessorTest.java:58-65
			File file = new File(dir, "legacy-array.bin");
			A3SFilesAccessor.write(new Object[] { "sync-node", "1" }, file);
			Object[] restored = (Object[]) A3SFilesAccessor.read(file);
			if (restored.length != 2 || !"sync-node".equals(restored[0]) || !"1".equals(restored[1])) {
				throw new HarnessException("Object[] differs after the round trip");
			}
			return "Object[] round trip";
		}
		}
	}

	/** The exception message and its causes on one tab-free line. */
	static String describe(Throwable t) {
		StringBuilder sb = new StringBuilder(t.getMessage() == null ? t.toString() : t.getMessage());
		Throwable cause = t.getCause();
		for (int depth = 0; cause != null && depth < 8; depth++, cause = cause.getCause()) {
			sb.append("\ncaused by: ").append(cause);
		}
		return oneLine(sb.toString());
	}

	static String oneLine(String text) {
		return text.replace("\r\n", "\n").replace('\r', '\n').replace("\n", " | ").replace('\t', ' ');
	}
}
