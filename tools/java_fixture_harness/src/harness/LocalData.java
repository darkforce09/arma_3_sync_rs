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
 * The local data set, built with domain setters and the GUI services. Values are non-default
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

	/**
	 * A separate {@code String} object for a value a user types. Java string literals are
	 * interned, so reusing one literal for two inputs would make {@code ObjectOutputStream} write
	 * a back-reference that a real client does not produce; the fixtures must only show the
	 * sharing Arma3Sync's own code creates.
	 */
	@SuppressWarnings("StringOperationCanBeSimplified")
	static String typed(String value) {
		return new String(value);
	}

	static void fillConfiguration(Configuration c) {
		c.setHeight(900);
		c.setWidth(1400);
		c.setViewModeTree(false);
		c.setProfileName(typed(EVENT_NIGHT));
		c.setServerName(typed("WOG #1"));
		c.setDefaultModset(typed(SAMPLE_HTTP));

		FavoriteServer full = new FavoriteServer();
		full.setDescription(typed("Alpha Server"));
		full.setIpAddress(typed("192.0.2.10"));
		full.setPort(2302);
		full.setPassword(typed("alpha-pass"));
		full.setSelected(true);
		full.setModsetName(typed(EVENT_NIGHT));
		full.setRepositoryName(typed(SAMPLE_HTTP));
		c.getFavoriteServers().add(full);
		FavoriteServer minimal = new FavoriteServer();
		minimal.setDescription(typed("Bravo Server"));
		minimal.setIpAddress(typed("192.0.2.20"));
		minimal.setPort(2402);
		c.getFavoriteServers().add(minimal);

		ExternalApplication app = new ExternalApplication();
		app.setName(typed("TeamSpeak 3"));
		app.setExecutablePath(typed(ROOT + "/tools/ts3client_linux_amd64"));
		app.setParameters(typed("-nosingleinstance"));
		app.setEnable(true);
		c.getExternalApplications().add(app);

		c.getAcreOptions().setTs3Path(typed(ROOT + "/ts3"));
		c.getAcreOptions().setAcrePluginPath(typed(ROOT + "/ts3/plugins/acre"));
		c.getAcreOptions().setAcreUserconfigPath(typed(ROOT + "/userconfig/acre"));
		c.getAcre2Options().setTs3Path(typed(ROOT + "/ts3"));
		c.getAcre2Options().setAcre2PluginPath(typed(ROOT + "/ts3/plugins/acre2"));
		c.getTfarOptions().setTs3Path(typed(ROOT + "/ts3"));
		c.getTfarOptions().setTfarPluginPath(typed(ROOT + "/ts3/plugins/tfar"));
		c.getTfarOptions().setTfarUserconfigPath(typed(ROOT + "/userconfig/tfar"));
		c.getRptOptions().setRptPath(typed(ROOT + "/rpt"));
		c.getBikeyExtractOptions().setSourceDirectoryPath(typed(ROOT + "/@Mod_A"));
		c.getBikeyExtractOptions().setTargetDirectoryPath(typed(ROOT + "/keys"));

		c.getAddonSearchDirectoryPaths().add(typed("/home/user/arma3/!Workshop"));
		c.getAddonSearchDirectoryPaths().add(typed("C:\\Games\\Arma 3"));
		fillLauncherOptions(c.getLauncherOptions());

		c.getProxy().setEnableProxy(true);
		c.getProxy().setProxyProtocol(AbstractProtocoleFactory.getProtocol(typed("proxy.example.org"), typed("1080"), typed("proxy-user"),
				typed("proxy-pass"), ProtocolType.SOCKS5, false));
	}

	static void fillLauncherOptions(LauncherOptions o) {
		o.setGameProfile(typed("Sample Player"));
		o.setShowScriptErrors(true);
		o.setNoPause(true);
		o.setWindowMode(true);
		o.setCheckSignatures(true);
		o.setMaxMemorySelection(typed("8192"));
		o.setCpuCountSelection(6);
		o.setExThreadsSelection(typed("7"));
		o.setMallocSelection(typed("system"));
		o.setEnableHT(true);
		o.setNoSplashScreen(true);
		o.setArma3ExePath(typed(ROOT + "/arma3_x64.exe"));
		o.setSteamExePath(typed(ROOT + "/steam.exe"));
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
		Profile profile = new Profile(typed(EVENT_NIGHT));
		profile.setAdditionalParameters(typed("-world=empty -noLogs"));
		profile.getAddonNamesByPriority().addAll(List.of(typed("@Mod_A"), typed("@mod_b"), typed("@gm")));
		profile.getAddonSearchDirectories().addAll(List.of(typed(ROOT), typed("/home/user/arma3/!Workshop")));
		profile.setLastAddedAddonSearchDirectory(typed(ROOT));
		fillLauncherOptions(profile.getLauncherOptions());

		TreeDirectory racine = profile.getTree();
		TreeDirectory repoGroup = new TreeDirectory(typed("Sample HTTP mods"), racine);
		repoGroup.setModsetType(ModsetType.REPOSITORY);
		repoGroup.setModsetRepositoryName(typed(SAMPLE_HTTP));
		repoGroup.setSelected(true);
		racine.addTreeNode(repoGroup);
		TreeLeaf modA = new TreeLeaf(typed("@Mod_A"), repoGroup);
		modA.setSelected(true);
		repoGroup.addTreeNode(modA);
		TreeLeaf modB = new TreeLeaf(typed("@mod_b"), repoGroup);
		modB.setOptional(true);
		repoGroup.addTreeNode(modB);
		TreeDirectory nested = new TreeDirectory(typed("Nested"), repoGroup);
		nested.setMarked(true);
		repoGroup.addTreeNode(nested);
		TreeLeaf nestedLeaf = new TreeLeaf(typed("@Nested"), nested);
		nestedLeaf.setSelected(true);
		nested.addTreeNode(nestedLeaf);

		TreeDirectory eventGroup = new TreeDirectory(typed("Night Op"), racine);
		eventGroup.setModsetType(ModsetType.EVENT);
		racine.addTreeNode(eventGroup);
		TreeLeaf gm = new TreeLeaf(typed("@gm"), eventGroup);
		gm.setSelected(true);
		gm.setWorkshopId(typed("3147580028"));
		eventGroup.addTreeNode(gm);

		TreeLeaf ws = new TreeLeaf(typed("WS"), racine);
		ws.setSelected(true);
		ws.setDlcAppId(typed("1681170"));
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
		service.createRepository(typed(SAMPLE_HTTP), typed("repo.example.org/arma3"), typed("80"), typed("anonymous"), typed(""), ProtocolType.HTTP,
				true);
		service.createRepository(typed(SAMPLE_FTP), typed("ftp.example.org/arma3"), typed("21"), typed("player"), typed("ftp-pass"), ProtocolType.FTP,
				true);
		service.createRepository(typed(SAMPLE_SFTP), typed("sftp.example.org/srv/arma3"), typed("22"), typed("sftp-user"), typed("sftp-pass"),
				ProtocolType.SFTP, false);
		for (String name : List.of(SAMPLE_HTTP, SAMPLE_FTP, SAMPLE_SFTP)) {
			fillRepository(service, name);
		}
		service.setRepositoryUploadProtocole(SAMPLE_FTP, typed("upload.example.org/arma3"), typed("990"), typed("uploader"), typed("upload-pass"),
				ProtocolType.FTPS, typed("30000"), typed("60000"), true);
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
		r.setDefaultDownloadLocation(typed(ROOT + "/downloads"));
		r.getMapEventsDownloadLocation().put(typed("Night Op"), typed(ROOT + "/downloads/night_op"));
		r.getHiddenFolderPath().add(typed("docs/hidden_a"));
		r.getHiddenFolderPath().add(typed("docs/hidden_b"));
		r.getMapFilesForSync().put(typed(ROOT + "/downloads/@Mod_A/addons/empty.txt"), new FileAttributes("0", 1704024000000L));
		r.getMapFilesForSync().put(typed(ROOT + "/downloads/@Mod_A/addons/mod_a.pbo"),
				new FileAttributes(typed("2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"), 1704024060000L));
		r.getMapFilesForBuild().put(typed(ROOT + "/@Mod_A/addons/empty.txt"), new FileAttributes("0", 1704024120000L));
		r.getMapFilesForBuild().put(typed(ROOT + "/@Mod_A/addons/mod_a.pbo"),
				new FileAttributes(typed("de9f2c7fd25e1b3afad3e85a0bd17d9b100db4b3"), 1704024180000L));
		r.setPath(typed(ROOT));
		r.setNumberOfConnections(2);
		r.setCompressed(true);
		r.setUsePartialFileTransfer(false);
		r.setAutoConfigURL(r.getProtocol().getProtocolType().getPrompt() + r.getProtocol().getUrl() + typed("/.a3s/autoconfig"));
		// GUI path: RepositoryService.java:1272-1282 with the configuration's first favorite server.
		service.setFavoriteServerToAutoconfig(r.getName(), List.of(0));
		r.getExcludedFilesFromBuild().add(typed(ROOT + "/excluded.txt"));
		r.getExcludedFoldersFromSync().add(typed(ROOT + "/docs/hidden_a"));
	}
}
