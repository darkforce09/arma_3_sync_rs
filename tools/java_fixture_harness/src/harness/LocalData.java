package harness;

import java.util.List;

import fr.soe.a3s.constant.CheckRepositoriesFrequency;
import fr.soe.a3s.constant.IconResize;
import fr.soe.a3s.constant.LookAndFeel;
import fr.soe.a3s.constant.MinimizationType;
import fr.soe.a3s.constant.ModsetType;
import fr.soe.a3s.constant.ProtocolType;
import fr.soe.a3s.constant.StartWithOS;
import fr.soe.a3s.dao.repository.RepositoryDAO;
import fr.soe.a3s.domain.AbstractProtocoleFactory;
import fr.soe.a3s.domain.Preferences;
import fr.soe.a3s.domain.Profile;
import fr.soe.a3s.domain.TreeDirectory;
import fr.soe.a3s.domain.TreeLeaf;
import fr.soe.a3s.domain.configration.Configuration;
import fr.soe.a3s.domain.configration.ExternalApplication;
import fr.soe.a3s.domain.configration.FavoriteServer;
import fr.soe.a3s.domain.configration.LauncherOptions;
import fr.soe.a3s.domain.repository.FileAttributes;
import fr.soe.a3s.domain.repository.Repository;
import fr.soe.a3s.service.RepositoryService;

/**
 * The B22 local data set, built with domain setters and the GUI services. Values are non-default
 * wherever a field has a default, and every absolute path is a fixed fake string.
 */
public final class LocalData {

	static final String ROOT = "/srv/arma3/SampleRepo";
	static final String EVENT_NIGHT = "Event Night";
	static final String SAMPLE_HTTP = "Sample HTTP";
	static final String SAMPLE_FTP = "Sample FTP";
	static final String SAMPLE_SFTP = "Sample SFTP";

	private LocalData() {
	}

	static void fillConfiguration(Configuration c) {
		c.setHeight(900);
		c.setWidth(1400);
		c.setViewModeTree(false);
		c.setProfileName(EVENT_NIGHT);
		c.setServerName("WOG #1");
		c.setDefaultModset(SAMPLE_HTTP);

		FavoriteServer full = new FavoriteServer();
		full.setDescription("Alpha Server");
		full.setIpAddress("192.0.2.10");
		full.setPort(2302);
		full.setPassword("alpha-pass");
		full.setSelected(true);
		full.setModsetName(EVENT_NIGHT);
		full.setRepositoryName(SAMPLE_HTTP);
		c.getFavoriteServers().add(full);
		FavoriteServer minimal = new FavoriteServer();
		minimal.setDescription("Bravo Server");
		minimal.setIpAddress("192.0.2.20");
		minimal.setPort(2402);
		c.getFavoriteServers().add(minimal);

		ExternalApplication app = new ExternalApplication();
		app.setName("TeamSpeak 3");
		app.setExecutablePath(ROOT + "/tools/ts3client_linux_amd64");
		app.setParameters("-nosingleinstance");
		app.setEnable(true);
		c.getExternalApplications().add(app);

		c.getAcreOptions().setTs3Path(ROOT + "/ts3");
		c.getAcreOptions().setAcrePluginPath(ROOT + "/ts3/plugins/acre");
		c.getAcreOptions().setAcreUserconfigPath(ROOT + "/userconfig/acre");
		c.getAcre2Options().setTs3Path(ROOT + "/ts3");
		c.getAcre2Options().setAcre2PluginPath(ROOT + "/ts3/plugins/acre2");
		c.getTfarOptions().setTs3Path(ROOT + "/ts3");
		c.getTfarOptions().setTfarPluginPath(ROOT + "/ts3/plugins/tfar");
		c.getTfarOptions().setTfarUserconfigPath(ROOT + "/userconfig/tfar");
		c.getRptOptions().setRptPath(ROOT + "/rpt");
		c.getBikeyExtractOptions().setSourceDirectoryPath(ROOT + "/@Mod_A");
		c.getBikeyExtractOptions().setTargetDirectoryPath(ROOT + "/keys");

		c.getAddonSearchDirectoryPaths().add("/home/user/arma3/!Workshop");
		c.getAddonSearchDirectoryPaths().add("C:\\Games\\Arma 3");
		fillLauncherOptions(c.getLauncherOptions());

		c.getProxy().setEnableProxy(true);
		c.getProxy().setProxyProtocol(AbstractProtocoleFactory.getProtocol("proxy.example.org", "1080", "proxy-user",
				"proxy-pass", ProtocolType.SOCKS5, false));
	}

	static void fillLauncherOptions(LauncherOptions o) {
		o.setGameProfile("Sample Player");
		o.setShowScriptErrors(true);
		o.setNoPause(true);
		o.setWindowMode(true);
		o.setCheckSignatures(true);
		o.setMaxMemorySelection("8192");
		o.setCpuCountSelection(6);
		o.setExThreadsSelection("7");
		o.setMallocSelection("system");
		o.setEnableHT(true);
		o.setNoSplashScreen(true);
		o.setArma3ExePath(ROOT + "/arma3_x64.exe");
		o.setSteamExePath(ROOT + "/steam.exe");
	}

	static void fillPreferences(Preferences p) {
		p.setLaunchPanelGameLaunch(MinimizationType.TRAY);
		p.setLaunchPanelMinimized(MinimizationType.TRAY);
		p.setLookAndFeel(LookAndFeel.LAF_HIFI);
		p.setIconResizeSize(IconResize.SIZE2);
		p.setStartWithOS(StartWithOS.ENABLED);
		p.setCheckRepositoriesFrequency(CheckRepositoriesFrequency.FREQ2);
		p.setPreferGitHubUpdates(true);
	}

	static Profile eventNightProfile() {
		Profile profile = new Profile(EVENT_NIGHT);
		profile.setAdditionalParameters("-world=empty -noLogs");
		profile.getAddonNamesByPriority().addAll(List.of("@Mod_A", "@mod_b", "@gm"));
		profile.getAddonSearchDirectories().addAll(List.of(ROOT, "/home/user/arma3/!Workshop"));
		profile.setLastAddedAddonSearchDirectory(ROOT);
		fillLauncherOptions(profile.getLauncherOptions());

		TreeDirectory racine = profile.getTree();
		TreeDirectory repoGroup = new TreeDirectory("Sample HTTP mods", racine);
		repoGroup.setModsetType(ModsetType.REPOSITORY);
		repoGroup.setModsetRepositoryName(SAMPLE_HTTP);
		repoGroup.setSelected(true);
		racine.addTreeNode(repoGroup);
		TreeLeaf modA = new TreeLeaf("@Mod_A", repoGroup);
		modA.setSelected(true);
		repoGroup.addTreeNode(modA);
		TreeLeaf modB = new TreeLeaf("@mod_b", repoGroup);
		modB.setOptional(true);
		repoGroup.addTreeNode(modB);
		TreeDirectory nested = new TreeDirectory("Nested", repoGroup);
		nested.setMarked(true);
		repoGroup.addTreeNode(nested);
		TreeLeaf nestedLeaf = new TreeLeaf("@Nested", nested);
		nestedLeaf.setSelected(true);
		nested.addTreeNode(nestedLeaf);

		TreeDirectory eventGroup = new TreeDirectory("Night Op", racine);
		eventGroup.setModsetType(ModsetType.EVENT);
		racine.addTreeNode(eventGroup);
		TreeLeaf gm = new TreeLeaf("@gm", eventGroup);
		gm.setSelected(true);
		gm.setWorkshopId("3147580028");
		eventGroup.addTreeNode(gm);

		TreeLeaf ws = new TreeLeaf("WS", racine);
		ws.setSelected(true);
		ws.setDlcAppId("1681170");
		racine.addTreeNode(ws);
		return profile;
	}

	/**
	 * Creates the three sample repositories through
	 * {@code RepositoryService.createRepository} (protocol defaults from
	 * {@code AbstractProtocoleFactory}) and fills them. The configuration must already hold the
	 * favorite servers.
	 */
	static void createRepositories(RepositoryService service) throws Exception {
		service.createRepository(SAMPLE_HTTP, "repo.example.org/arma3", "80", "anonymous", "", ProtocolType.HTTP,
				true);
		service.createRepository(SAMPLE_FTP, "ftp.example.org/arma3", "21", "player", "ftp-pass", ProtocolType.FTP,
				true);
		service.createRepository(SAMPLE_SFTP, "sftp.example.org/srv/arma3", "22", "sftp-user", "sftp-pass",
				ProtocolType.SFTP, false);
		for (String name : List.of(SAMPLE_HTTP, SAMPLE_FTP, SAMPLE_SFTP)) {
			fillRepository(service, name);
		}
		service.setRepositoryUploadProtocole(SAMPLE_FTP, "upload.example.org/arma3", "990", "uploader", "upload-pass",
				ProtocolType.FTPS, "30000", "60000", true);
		Repository ftp = repository(SAMPLE_FTP);
		ftp.setUploadCompressedPboFilesOnly(true);
		ftp.setParallelUploadConnections(6);
	}

	static Repository repository(String name) {
		return new RepositoryDAO().getMap().get(name);
	}

	private static void fillRepository(RepositoryService service, String name) {
		Repository r = repository(name);
		r.setNotify(true);
		r.setAuto(true);
		r.setExactMatch(true);
		r.setNoAutoDiscover(false);
		r.setRevision(7);
		r.setNumberOfClientConnections(3);
		r.setNumberOfRepositoryCheckConnections(2);
		r.setMaximumClientDownloadSpeed(1.5);
		r.setDefaultDownloadLocation(ROOT + "/downloads");
		r.getMapEventsDownloadLocation().put("Night Op", ROOT + "/downloads/night_op");
		r.getHiddenFolderPath().add("docs/hidden_a");
		r.getHiddenFolderPath().add("docs/hidden_b");
		r.getMapFilesForSync().put(ROOT + "/downloads/@Mod_A/addons/empty.txt", new FileAttributes("0", 1704024000000L));
		r.getMapFilesForSync().put(ROOT + "/downloads/@Mod_A/addons/mod_a.pbo",
				new FileAttributes("2fd4e1c67a2d28fced849ee1bb76e7391b93eb12", 1704024060000L));
		r.getMapFilesForBuild().put(ROOT + "/@Mod_A/addons/empty.txt", new FileAttributes("0", 1704024120000L));
		r.getMapFilesForBuild().put(ROOT + "/@Mod_A/addons/mod_a.pbo",
				new FileAttributes("de9f2c7fd25e1b3afad3e85a0bd17d9b100db4b3", 1704024180000L));
		r.setPath(ROOT);
		r.setNumberOfConnections(2);
		r.setCompressed(true);
		r.setUsePartialFileTransfer(false);
		r.setAutoConfigURL(r.getProtocol().getProtocolType().getPrompt() + r.getProtocol().getUrl() + "/.a3s/autoconfig");
		// GUI path: RepositoryService.java:1272-1282 with the configuration's first favorite server.
		service.setFavoriteServerToAutoconfig(name, List.of(0));
		r.getExcludedFilesFromBuild().add(ROOT + "/excluded.txt");
		r.getExcludedFoldersFromSync().add(ROOT + "/docs/hidden_a");
	}
}
