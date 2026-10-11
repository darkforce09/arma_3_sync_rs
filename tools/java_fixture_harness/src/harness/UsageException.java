package harness;

/** A command-line usage error: the command exits with code 2. */
public class UsageException extends Exception {

	private static final long serialVersionUID = 1L;

	public UsageException(String message) {
		super(message);
	}
}
