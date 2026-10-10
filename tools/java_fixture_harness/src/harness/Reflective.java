package harness;

/**
 * Classes Arma3Sync only names in strings. {@code jazsync/JarsyncProvider.java} registers
 * {@code MessageDigest.MD4} by class name, so {@code javac -implicit:class} would not compile it;
 * these literals pull both digests into {@code build/classes}.
 */
final class Reflective {

	static final Class<?>[] CLASSES = { fr.soe.a3s.jazsync.MD4.class, fr.soe.a3s.jazsync.BrokenMD4.class };

	private Reflective() {
	}
}
