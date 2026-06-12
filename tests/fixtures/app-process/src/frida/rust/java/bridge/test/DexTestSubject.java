package frida.rust.java.bridge.test;

public final class DexTestSubject {
    private DexTestSubject() {
    }

    public static int answer() {
        return 4242;
    }

    public static String message() {
        return "dex-only-test";
    }
}
