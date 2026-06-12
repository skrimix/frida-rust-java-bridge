package frida.rust.java.bridge.test;

public final class AppProcessTest {
    private AppProcessTest() {
    }

    public static void main(String[] args) {
        System.load("/data/local/tmp/frida-rust-java-bridge/libfrida_rust_java_bridge_art_selftest.so");

        String result = nativeRun(AppProcessTest.class.getClassLoader());
        if (!"ok".equals(result)) {
            System.err.println("app_process_test: " + result);
            throw new RuntimeException(result);
        }

        System.out.println("app_process_test: ok");
        Runtime.getRuntime().halt(0);
    }

    private static native String nativeRun(ClassLoader loader);
}
