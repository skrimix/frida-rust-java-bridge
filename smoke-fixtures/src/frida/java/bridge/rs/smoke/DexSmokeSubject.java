package frida.java.bridge.rs.smoke;

public final class DexSmokeSubject {
    private DexSmokeSubject() {
    }

    public static int answer() {
        return 4242;
    }

    public static String message() {
        return "dex-only-smoke";
    }
}
