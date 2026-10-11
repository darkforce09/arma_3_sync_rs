package harness;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;

import static harness.LocalData.typed;

import fr.soe.a3s.constant.ProtocolType;
import fr.soe.a3s.dao.ConfigurationDAO;
import fr.soe.a3s.dao.FileAccessMethods;
import fr.soe.a3s.domain.configration.FavoriteServer;
import fr.soe.a3s.domain.repository.Repository;
import fr.soe.a3s.dto.EventDTO;
import fr.soe.a3s.dto.RepositoryDTO;
import fr.soe.a3s.service.RepositoryService;

/**
 * Built repositories ({@code java_built/}): the GUI build flow run three times per variant
 * (initial, unchanged rebuild in the same JVM, modified rebuild in a new JVM), with snapshots of
 * the repository tree, the upload serialization and identity dumps.
 */
public final class BuiltFixtures {

	static final List<String> VARIANTS = List.of("http_plain", "http_zip", "ftp_plain", "ftp_zip");
	static final String FOLDER = "SampleRepo";
	static final String[] STEPS = { "1_initial", "2_rebuild_nochange", "3_modified" };
	static final long[] BUILD_DATES = { 1704067200000L, 1704070800000L, 1704153600000L };
	static final long STEP1_MTIME = 1704024000000L;
	static final long STEP3_MTIME = 1704110400000L;
	static final String[] A3S_FILES = { "sync", "serverinfo", "changelogs", "autoconfig", "events" };

	/** Step 1 tree: path and size; -1 marks a directory. */
	static final Object[][] STEP1_TREE = { { "@Mod_A/addons/mod_a.pbo", 20000 },
			{ "@Mod_A/addons/mod_a.pbo.bisign", 300 }, { "@Mod_A/addons/empty.txt", 0 }, { "@Mod_A/mod.cpp", 120 },
			{ "@mod_b/Addons/mod_b.ebo", 9000 }, { "@mod_b/Addons/empty2.txt", 0 },
			{ "Collection/@Nested/addons/nested.pbo", 5000 }, { "docs/readme.txt", 200 },
			{ "docs/hidden_a/secret.txt", 50 }, { "docs/hidden_b/secret2.txt", 50 }, { "excluded.txt", 40 },
			{ "notes.a3s.txt", 30 }, { "readme.zsync.txt", 30 }, { "emptydir", -1 } };

	private BuiltFixtures() {
	}

	static String repositoryName(String variant) {
		return "Sample " + variant;
	}

	static boolean isHttp(String variant) {
		return variant.startsWith("http");
	}

	static boolean isZip(String variant) {
		return variant.endsWith("_zip");
	}

	/** The two configuration favorite servers, in the order {@code Configuration.getFavoriteServers()} sorts them. */
	static List<FavoriteServer> favoriteServers() {
		FavoriteServer alpha = new FavoriteServer();
		alpha.setDescription(typed("Alpha Server"));
		alpha.setIpAddress(typed("192.0.2.10"));
		alpha.setPort(2302);
		alpha.setPassword(typed("alpha-pass"));
		alpha.setSelected(true);
		alpha.setModsetName(typed("Night Op"));
		FavoriteServer bravo = new FavoriteServer();
		bravo.setDescription(typed("Bravo Server"));
		bravo.setIpAddress(typed("192.0.2.20"));
		bravo.setPort(2402);
		bravo.setModsetName(typed("Training"));
		return List.of(alpha, bravo);
	}

	// ------------------------------------------------------------------ parent

	static void generate(Path out, Path work) throws Exception {
		Path built = out.resolve("java_built");
		for (String variant : VARIANTS) {
			Path variantOut = built.resolve(variant);
			Path variantWork = work.resolve(variant);
			Map<String, String> properties = Main.perUserProperties(variantWork);
			Main.runChild(null, properties, "child-built", variant, "initial", variantWork.toString(),
					variantOut.toString());
			Main.runChild(null, properties, "child-built", variant, "modified", variantWork.toString(),
					variantOut.toString());
			for (String step : STEPS) {
				dumpStep(variantOut.resolve(step));
			}
			Io.writeString(variantOut.resolve("scenario.json"), scenario(variant, variantOut));
			Io.writeString(variantOut.resolve("generated_files.json"), generatedFiles(variant, variantOut));
		}
		EventsSession.generate(built.resolve("events_session"), work.resolve("events_session"));
	}

	/** Identity dumps of the five {@code .a3s} files and of the uploaded files. */
	private static void dumpStep(Path stepDir) throws Exception {
		for (String name : A3S_FILES) {
			Path a3s = stepDir.resolve("repo").resolve(".a3s").resolve(name);
			if (Files.exists(a3s)) {
				Io.writeString(stepDir.resolve("dumps").resolve(name + ".json"),
						JsonDump.dump(Verify.readChecked(a3s), false));
			}
			Path uploaded = stepDir.resolve("uploaded").resolve(name);
			if (Files.exists(uploaded)) {
				Object object = fr.soe.a3s.dao.A3SFilesAccessor.read(uploaded.toFile());
				Io.writeString(stepDir.resolve("dumps").resolve("uploaded_" + name + ".json"),
						JsonDump.dump(object, false));
			}
		}
	}

	/** {@code scenario.json}, derived from the snapshots. */
	private static String scenario(String variant, Path variantOut) throws Exception {
		Json.Obj protocol = Json.obj().put("type", isHttp(variant) ? "HTTP" : "FTP")
				.put("url", isHttp(variant) ? "repo.example.org/arma3" : "ftp.example.org/arma3")
				.put("port", isHttp(variant) ? "80" : "21").put("login", "anonymous").put("password", "");
		List<Object> favorites = new ArrayList<Object>();
		for (FavoriteServer f : favoriteServers()) {
			favorites.add(Json.obj().put("name", f.getDescription()).put("ip_address", f.getIpAddress())
					.put("port", f.getPort()).put("password", f.getPassword()).put("selected", f.isSelected())
					.put("modset_name", f.getModsetName()).put("repository_name", repositoryName(variant)));
		}
		List<Object> steps = new ArrayList<Object>();
		for (int i = 0; i < STEPS.length; i++) {
			Path repo = variantOut.resolve(STEPS[i]).resolve("repo");
			List<Object> files = new ArrayList<Object>();
			for (String rel : sourceFiles(repo)) {
				Path file = repo.resolve(rel);
				files.add(Json.obj().put("path", rel).put("size", Files.size(file))
						.put("mtime_ms", Files.getLastModifiedTime(file).toMillis())
						.put("sha1", FileAccessMethods.computeSHA1(file.toFile())));
			}
			List<Object> dirs = new ArrayList<Object>(Io.emptyDirectories(repo));
			steps.add(Json.obj().put("name", STEPS[i]).put("jvm", i == 1 ? "same" : "new")
					.put("build_date_ms", BUILD_DATES[i]).put("files", files).put("dirs", dirs)
					.put("input_events", i == 2 ? STEPS[2] + "/input/events" : null));
		}
		Json.Obj scenario = Json.obj().put("variant", variant).put("repository_name", repositoryName(variant))
				.put("repository_folder", FOLDER).put("protocol", protocol).put("compressed", isZip(variant))
				.put("upload_compressed_pbo_files_only", isZip(variant))
				.put("use_partial_file_transfer", !variant.equals("http_zip")).put("number_of_connections", 3)
				.put("excluded_files", Json.list("excluded.txt"))
				.put("hidden_folders", Json.list("docs/hidden_a", "docs/hidden_b")).put("favorite_servers", favorites)
				.put("scan_order", "sorted-by-name").put("steps", steps);
		return Json.document(scenario);
	}

	/**
	 * {@code generated_files.json}: every snapshot file outside {@code .a3s/} that is not a source
	 * file ({@code .zsync}, {@code .pbo.zip}, {@code .ebo.zip}) with its size and (pinned) mtime, so a
	 * checkout can restore the mtimes that {@code .zsync} headers and the next build depend on.
	 * {@code sync_leaf} applies Java's own exclusion rule ({@code RepositoryBuildProcessor.java:487-489}):
	 * a generated {@code .ebo.zip} is hashed by the next build.
	 */
	private static String generatedFiles(String variant, Path variantOut) throws IOException {
		List<Object> steps = new ArrayList<Object>();
		for (String step : STEPS) {
			Path repo = variantOut.resolve(step).resolve("repo");
			List<String> sources = sourceFiles(repo);
			List<Object> files = new ArrayList<Object>();
			for (String rel : Io.files(repo)) {
				if (rel.startsWith(".a3s/") || sources.contains(rel)) {
					continue;
				}
				String name = repo.resolve(rel).getFileName().toString();
				boolean leaf = !name.contains(".zsync") && !name.contains(".pbo.zip") && !name.contains(".a3s");
				// A .zsync file's own mtime is the wall clock and nothing reads it: null.
				Long mtime = name.endsWith(".zsync") ? null : Files.getLastModifiedTime(repo.resolve(rel)).toMillis();
				files.add(Json.obj().put("path", rel).put("size", Files.size(repo.resolve(rel))).put("mtime_ms", mtime)
						.put("sync_leaf", leaf));
			}
			steps.add(Json.obj().put("name", step).put("files", files));
		}
		return Json.document(Json.obj().put("variant", variant).put("steps", steps));
	}

	/** Source files of a snapshot: no {@code .a3s/} content, no {@code .zsync}, no generated {@code .zip}. */
	static List<String> sourceFiles(Path repo) throws IOException {
		List<String> files = new ArrayList<String>();
		for (String rel : Io.files(repo)) {
			if (rel.startsWith(".a3s/") || rel.endsWith(".zsync") || rel.endsWith(".zip")) {
				continue;
			}
			files.add(rel);
		}
		return files;
	}

	// ------------------------------------------------------------------ child

	/** {@code child-built <variant> <initial|modified> <work> <variant out>}. */
	static void child(List<String> args) throws Exception {
		if (args.size() != 4) {
			throw new UsageException("usage: child-built <variant> <initial|modified> <work> <out>");
		}
		String variant = args.get(0);
		if (!VARIANTS.contains(variant)) {
			throw new UsageException("unknown built variant: " + variant);
		}
		Path work = Paths.get(args.get(2)).toAbsolutePath().normalize();
		Path out = Paths.get(args.get(3)).toAbsolutePath().normalize();
		Path repo = work.resolve(FOLDER);
		String name = repositoryName(variant);
		RepositoryService service = new RepositoryService();
		switch (args.get(1)) {
		case "initial":
			createSampleTree(repo);
			setUpRepository(service, variant, repo);
			build(name, repo, BUILD_DATES[0]);
			snapshot(service, name, repo, out.resolve(STEPS[0]));
			build(name, repo, BUILD_DATES[1]);
			snapshot(service, name, repo, out.resolve(STEPS[1]));
			break;
		case "modified":
			// New JVM: the repository comes back from the encrypted file the service build wrote.
			service.readAll();
			if (LocalData.repository(name) == null) {
				throw new HarnessException("repository " + name + " not found after readAll");
			}
			modifyTree(repo);
			writeStep3Events(service, name);
			Path input = out.resolve(STEPS[2]).resolve("input").resolve("events");
			Files.createDirectories(input.getParent());
			Files.copy(repo.resolve(".a3s").resolve("events"), input, StandardCopyOption.COPY_ATTRIBUTES);
			build(name, repo, BUILD_DATES[2]);
			snapshot(service, name, repo, out.resolve(STEPS[2]));
			break;
		default:
			throw new UsageException("unknown child-built stage: " + args.get(1));
		}
	}

	/** The step 1 tree, contents from {@link SampleContent}, pinned mtimes. */
	static void createSampleTree(Path repo) throws Exception {
		String lower = repo.toString().toLowerCase(Locale.ROOT);
		String needle = "/" + FOLDER.toLowerCase(Locale.ROOT);
		if (lower.indexOf(needle) < 0 || lower.indexOf(needle) != lower.lastIndexOf(needle)) {
			// RepositoryBuildProcessor.java:160-167 uses the first indexOf of /<main folder>.
			throw new HarnessException("the repository path must contain " + needle + " exactly once: " + repo);
		}
		Set<String> hidden = new HashSet<String>();
		for (String path : List.of("docs/hidden_a", "docs/hidden_b")) {
			int h = path.hashCode();
			hidden.add(Integer.toString((h ^ (h >>> 16)) & 15));
		}
		if (hidden.size() != 2) {
			throw new HarnessException("docs/hidden_a and docs/hidden_b share a HashSet bucket");
		}
		List<String> paths = new ArrayList<String>();
		for (Object[] entry : STEP1_TREE) {
			if ((Integer) entry[1] >= 0) {
				paths.add((String) entry[0]);
			}
		}
		paths.sort(String::compareTo);
		for (Object[] entry : STEP1_TREE) {
			String path = (String) entry[0];
			int size = (Integer) entry[1];
			if (size < 0) {
				Files.createDirectories(repo.resolve(path));
			} else {
				Io.writeBytes(repo.resolve(path), SampleContent.bytes(path, size, 0),
						STEP1_MTIME + 60000L * paths.indexOf(path));
			}
		}
	}

	/** Repository creation and settings through {@code RepositoryService}. */
	static void setUpRepository(RepositoryService service, String variant, Path repo) throws Exception {
		String name = repositoryName(variant);
		List<FavoriteServer> configured = new ConfigurationDAO().getConfiguration().getFavoriteServers();
		configured.clear();
		configured.addAll(favoriteServers());
		if (isHttp(variant)) {
			service.createRepository(name, typed("repo.example.org/arma3"), typed("80"), typed("anonymous"), typed(""), ProtocolType.HTTP, false);
		} else {
			service.createRepository(name, typed("ftp.example.org/arma3"), typed("21"), typed("anonymous"), typed(""), ProtocolType.FTP, false);
		}
		if (isZip(variant)) {
			service.setCompressed(name, true);
			service.setUploadCompressedPboFilesOnly(name, true);
		}
		if (variant.equals("http_zip")) {
			service.setUsePartialFileTransfer(name, false);
		}
		service.setExcludedFilesPathFromBuild(name, List.of(repo.resolve("excluded.txt").toString()));
		service.setExcludedFoldersFromSync(name,
				List.of(repo.resolve("docs/hidden_a").toString(), repo.resolve("docs/hidden_b").toString()));
		service.setNumberOfConnections(name, 3);
		// GUI path (BuildRepositoryOptionsDialog.java:616): the name comes from the repository DTO,
		// which holds the repository's own String (ObjectDTOtransformer.java:222).
		String nameFromDto = null;
		for (RepositoryDTO dto : service.getRepositories()) {
			if (dto.getName().equals(name)) {
				nameFromDto = dto.getName();
			}
		}
		if (nameFromDto == null) {
			throw new HarnessException("repository " + name + " missing from getRepositories()");
		}
		service.setFavoriteServerToAutoconfig(nameFromDto, List.of(0, 1));
	}

	/**
	 * One GUI build through {@code service/administration/RepositoryBuildProcessor}, followed
	 * by the zip post-processing of {@link #normalizeNewZips}.
	 */
	static void build(String name, Path repo, long buildDate) throws Exception {
		Map<String, Long> zipsBefore = zipMtimes(repo);
		HarnessClock.set(buildDate);
		runBuild(name, repo);
		normalizeNewZips(repo, zipsBefore, buildDate);
	}

	private static Map<String, Long> zipMtimes(Path repo) throws IOException {
		Map<String, Long> zips = new LinkedHashMap<String, Long>();
		for (String rel : Io.files(repo)) {
			if (rel.endsWith(".pbo.zip") || rel.endsWith(".ebo.zip")) {
				zips.put(rel, Files.getLastModifiedTime(repo.resolve(rel)).toMillis());
			}
		}
		return zips;
	}

	/**
	 * Zips written by this build carry the wall-clock time in their DOS header and file mtime
	 * ({@code ZipDAO.java:135-136}). Because {@code RepositoryBuildProcessor.java:487} only excludes
	 * {@code .pbo.zip}, a generated {@code .ebo.zip} becomes a sync leaf on the next build (Java
	 * quirk, kept), so its bytes and mtime must be pinned in the working tree, not only in the
	 * snapshot: DOS time 1980-01-01 00:00 and mtime = the build date.
	 */
	private static void normalizeNewZips(Path repo, Map<String, Long> before, long buildDate) throws Exception {
		for (Map.Entry<String, Long> zip : zipMtimes(repo).entrySet()) {
			if (!zip.getValue().equals(before.get(zip.getKey()))) {
				Path path = repo.resolve(zip.getKey());
				ZipNormalizer.normalize(path);
				Files.setLastModifiedTime(path, java.nio.file.attribute.FileTime.fromMillis(buildDate));
			}
		}
	}

	private static void runBuild(String name, Path repo) throws Exception {
		List<Exception> errors = new ArrayList<Exception>();
		boolean[] ended = { false };
		fr.soe.a3s.service.administration.RepositoryBuildProcessor processor = new fr.soe.a3s.service.administration.RepositoryBuildProcessor(
				name, repo.toString());
		processor.addObserverText(text -> System.err.println("build " + name + ": " + text));
		processor.addObserverCountProgress(value -> System.err.println("build " + name + ": " + value + "%"));
		processor.addObserverEnd(() -> ended[0] = true);
		processor.addObserverError(errors::addAll);
		processor.run();
		if (!errors.isEmpty()) {
			for (Exception e : errors) {
				e.printStackTrace();
			}
			throw new HarnessException("build of " + name + " failed: " + errors.get(0), errors.get(0));
		}
		if (!ended[0]) {
			throw new HarnessException("build of " + name + " did not report its end");
		}
	}

	/** Repository copy with normalized zips, plus the upload serialization. */
	static void snapshot(RepositoryService service, String name, Path repo, Path stepOut) throws Exception {
		Path repoOut = stepOut.resolve("repo");
		Io.copyTree(repo, repoOut);
		for (String rel : Io.files(repoOut)) {
			if (rel.endsWith(".pbo.zip") || rel.endsWith(".ebo.zip")) {
				ZipNormalizer.normalize(repoOut.resolve(rel));
			}
		}
		// Mirrors service/RepositoryService.java:1394-1420 and ConnectionUploadProcessor.java:89-90, 214-227
		service.readLocalyBuildedRepository(name);
		Repository repository = LocalData.repository(name);
		repository.getLocalServerInfo().setCompressedPboFilesOnly(repository.isUploadCompressedPboFilesOnly());
		Path uploaded = stepOut.resolve("uploaded");
		Io.writeUploadStyle(repository.getLocalSync(), uploaded.resolve("sync"));
		Io.writeUploadStyle(repository.getLocalServerInfo(), uploaded.resolve("serverinfo"));
		Io.writeUploadStyle(repository.getLocalChangelogs(), uploaded.resolve("changelogs"));
		Io.writeUploadStyle(repository.getLocalAutoConfig(), uploaded.resolve("autoconfig"));
		if (repository.getLocalEvents() != null) {
			Io.writeUploadStyle(repository.getLocalEvents(), uploaded.resolve("events"));
		}
	}

	/** Step 3 file changes; new and rewritten files get step 3 mtimes. */
	private static void modifyTree(Path repo) throws Exception {
		Io.deleteTree(repo.resolve("Collection/@Nested"));
		Map<String, Integer> changes = new LinkedHashMap<String, Integer>();
		changes.put("@Mod_A/addons/mod_a.pbo", 21000);
		changes.put("@Mod_D/addons/mod_d.pbo", 6000);
		changes.put("@mod_b/Addons/new_empty.txt", 0);
		List<String> all = new ArrayList<String>(sourceFiles(repo));
		for (String path : changes.keySet()) {
			if (!all.contains(path)) {
				all.add(path);
			}
		}
		all.sort(String::compareTo);
		for (Map.Entry<String, Integer> change : changes.entrySet()) {
			String path = change.getKey();
			Io.writeBytes(repo.resolve(path), SampleContent.bytes(path, change.getValue(), 1),
					STEP3_MTIME + 60000L * all.indexOf(path));
		}
		if (!Files.isDirectory(repo.resolve("Collection"), LinkOption.NOFOLLOW_LINKS)) {
			throw new HarnessException("Collection/ vanished");
		}
	}

	/** Step 3 events through the GUI flow: addEvent then saveEvent per event, then writeEvents. */
	private static void writeStep3Events(RepositoryService service, String name) throws Exception {
		EventDTO night = new EventDTO();
		night.setName(typed("Night Op"));
		night.setDescription(typed("Night operation"));
		night.getAddonNames().put(typed("@Mod_A"), true);
		night.getAddonNames().put(typed("@mod_b"), false);
		night.getAddonNames().put(typed("@Gone"), true);
		night.getDlcNames().put(typed("WS"), true);
		night.getDlcNames().put(typed("GM"), false);
		night.getUserconfigFolderNames().put(typed("@Mod_A"), true);

		EventDTO training = new EventDTO();
		training.setName(typed("Training"));
		training.setDescription(typed("Weekly training"));
		Map<String, Boolean> addons = new HashMap<String, Boolean>();
		addons.put(typed("@Mod_A"), true);
		addons.put(typed("@mod_b"), true);
		addons.put(typed("@Mod_D"), false);
		for (int i = 1; i <= 9; i++) {
			addons.put(String.format(typed("@Unknown%02d"), i), i % 2 == 0);
		}
		training.getAddonNames().putAll(addons);
		for (String key : List.of(typed("@Mod_A"), typed("@mod_b"), typed("@Mod_D"), typed("@Cfg04"), typed("@Cfg05"), typed("@Cfg06"))) {
			training.getUserconfigFolderNames().put(key, !key.equals(typed("@Cfg05")));
		}

		for (EventDTO event : List.of(night, training)) {
			service.addEvent(name, event);
			service.saveEvent(name, event);
		}
		service.writeEvents(name);
	}
}
