package harness;

import java.io.File;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import fr.soe.a3s.dao.A3SFilesAccessor;
import fr.soe.a3s.dao.ConfigurationDAO;
import fr.soe.a3s.dao.EncryptionProvider;
import fr.soe.a3s.dao.PreferencesDAO;
import fr.soe.a3s.dao.ProfileDAO;
import fr.soe.a3s.domain.Preferences;
import fr.soe.a3s.domain.Profile;
import fr.soe.a3s.domain.configration.Configuration;
import fr.soe.a3s.domain.repository.Repository;
import fr.soe.a3s.service.CommonService;
import fr.soe.a3s.service.RepositoryService;

/**
 * Local client files ({@code java_local/}): per-user and portable layouts written by the real DAOs,
 * plain (unencrypted) repositories, the historical variants and the autoconfig export.
 */
public final class LocalFixtures {

	static final List<String> VARIANTS = List.of("repository_pre_connections", "prefs_pre_github", "cfg_with_aia",
			"treeleaf_pre_workshop");

	private LocalFixtures() {
	}

	/** Parent side: runs the children, collects their files and dumps every file. */
	static void generate(Path out, Path work) throws Exception {
		Path local = out.resolve("java_local");
		Files.createDirectories(local);

		Path perUser = work.resolve("peruser");
		Main.runChild(null, Main.perUserProperties(perUser), "child-local", "peruser", perUser.toString());
		for (String dir : List.of("config", "data")) {
			Io.copyTree(perUser.resolve(dir), local.resolve("peruser").resolve(dir));
		}
		Io.copyTree(perUser.resolve("plain"), local.resolve("plain"));
		Io.copyTree(perUser.resolve("export"), local.resolve("export"));

		Path portable = work.resolve("portable");
		Path portableRoot = portable.resolve("root");
		Files.createDirectories(portableRoot.resolve("resources"));
		Map<String, String> portableProperties = new LinkedHashMap<String, String>(Main.perUserProperties(portable));
		portableProperties.remove("a3s.configPath");
		portableProperties.remove("a3s.dataPath");
		portableProperties.remove("a3s.cachePath");
		portableProperties.put("a3s.portable", "true");
		portableProperties.put("a3s.installationPath", portableRoot.toString());
		Main.runChild(null, portableProperties, "child-local", "portable", portable.toString());
		for (String dir : List.of("resources", "profiles")) {
			Io.copyTree(portableRoot.resolve(dir), local.resolve("portable").resolve(dir));
		}

		Path variants = local.resolve("variants");
		Files.createDirectories(variants);
		for (String variant : VARIANTS) {
			Path variantWork = work.resolve("variant_" + variant);
			Main.runChild(variant, Main.perUserProperties(variantWork), "child-variant", variant, variants.toString());
		}

		// Identity dumps by this (parent) JVM with the current classes.
		Path dumps = local.resolve("dumps");
		for (String rel : Io.files(local)) {
			if (rel.startsWith("dumps/")) {
				continue;
			}
			Object object = Verify.readChecked(local.resolve(rel));
			Io.writeString(dumps.resolve(rel + ".json"), JsonDump.dump(object, false));
		}
	}

	/** Child side: {@code child-local <peruser|portable> <work>}. */
	static void child(List<String> args) throws Exception {
		if (args.size() != 2) {
			throw new UsageException("usage: child-local <peruser|portable> <work>");
		}
		String mode = args.get(0);
		Path work = Paths.get(args.get(1));

		// The configuration is written first: ProfileDAO.write uses a single-level mkdir (ProfileDAO.java:76).
		ConfigurationDAO configurationDAO = new ConfigurationDAO();
		LocalData.fillConfiguration(configurationDAO.getConfiguration());
		configurationDAO.write();
		PreferencesDAO preferencesDAO = new PreferencesDAO();
		LocalData.fillPreferences(preferencesDAO.getPreferences());
		preferencesDAO.write();

		ProfileDAO profileDAO = new ProfileDAO();
		Profile defaultProfile = new Profile("Default");
		Profile eventNight = LocalData.eventNightProfile();
		profileDAO.getMap().put(defaultProfile.getName(), defaultProfile);
		profileDAO.getMap().put(eventNight.getName(), eventNight);
		profileDAO.write(defaultProfile);
		profileDAO.write(eventNight);

		RepositoryService repositoryService = new RepositoryService();
		LocalData.createRepositories(repositoryService);
		for (String name : List.of(LocalData.SAMPLE_HTTP, LocalData.SAMPLE_FTP, LocalData.SAMPLE_SFTP)) {
			repositoryService.write(name);
		}
		if (mode.equals("portable")) {
			return;
		}
		if (!mode.equals("peruser")) {
			throw new UsageException("unknown child-local mode: " + mode);
		}

		// Legacy-named byte copy (RepositoryDAO.java:322-328 naming).
		Path repositories = work.resolve("data").resolve("Arma3Sync").resolve("repositories");
		Path ftpFile = null;
		for (String rel : Io.files(repositories)) {
			if (rel.startsWith("Sample_FTP__")) {
				ftpFile = repositories.resolve(rel);
			}
		}
		if (ftpFile == null) {
			throw new HarnessException("no Sample_FTP__*.a3s.repository in " + repositories);
		}
		Files.copy(ftpFile, repositories.resolve("SampleFTP.a3s.repository"));

		// Plain repositories.
		for (String name : List.of(LocalData.SAMPLE_HTTP, LocalData.SAMPLE_FTP, LocalData.SAMPLE_SFTP)) {
			Path plain = work.resolve("plain").resolve(name.replace(' ', '_') + ".repository.gz");
			Files.createDirectories(plain.getParent());
			A3SFilesAccessor.write(LocalData.repository(name), plain.toFile());
		}

		// Autoconfig export, after the profile files (CommonService.java:62 mutates the cached profile).
		Path export = work.resolve("export");
		Files.createDirectories(export);
		new CommonService().exportAutoConfig(List.of(LocalData.EVENT_NIGHT, "Default"), List.of(0),
				List.of(LocalData.SAMPLE_HTTP), export.toFile());
	}

	/** Child side: {@code child-variant <variant> <out dir>}, run with the variant class first. */
	static void childVariant(List<String> args) throws Exception {
		if (args.size() != 2) {
			throw new UsageException("usage: child-variant <variant> <out dir>");
		}
		String variant = args.get(0);
		File out = new File(args.get(1));
		switch (variant) {
		case "repository_pre_connections": {
			LocalData.fillConfiguration(new ConfigurationDAO().getConfiguration());
			LocalData.createRepositories(new RepositoryService());
			Repository ftp = LocalData.repository(LocalData.SAMPLE_FTP);
			A3SFilesAccessor.write(ftp, EncryptionProvider.getEncryptionCipher(),
					new File(out, variant + ".a3s.repository"));
			A3SFilesAccessor.write(ftp, new File(out, variant + ".repository.gz"));
			break;
		}
		case "prefs_pre_github": {
			Preferences preferences = new Preferences();
			LocalData.fillPreferences(preferences);
			A3SFilesAccessor.write(preferences, new File(out, variant + ".a3s.prefs"));
			break;
		}
		case "cfg_with_aia": {
			Configuration configuration = new Configuration();
			LocalData.fillConfiguration(configuration);
			Field field = Configuration.class.getDeclaredField("aiaOptions");
			field.setAccessible(true);
			Object aia = field.get(configuration);
			String[][] values = { { "arma2Path", "C:\\Games\\ArmA 2" }, { "armaPath", "C:\\Games\\ArmA" },
					{ "tohPath", "C:\\Games\\Take On Helicopters" }, { "arma2OAPath", "C:\\Games\\ArmA 2 OA" },
					{ "allinArmaPath", "C:\\Games\\All in Arma" } };
			for (String[] value : values) {
				Field f = aia.getClass().getDeclaredField(value[0]);
				f.setAccessible(true);
				f.set(aia, value[1]);
			}
			A3SFilesAccessor.write(configuration, new File(out, variant + ".a3s.cfg"));
			break;
		}
		case "treeleaf_pre_workshop":
			A3SFilesAccessor.write(LocalData.eventNightProfile(), new File(out, variant + ".a3s.profile"));
			break;
		default:
			throw new UsageException("unknown variant: " + variant);
		}
	}
}
