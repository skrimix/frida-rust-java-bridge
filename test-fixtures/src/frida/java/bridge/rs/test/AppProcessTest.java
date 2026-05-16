package frida.java.bridge.rs.test;

public final class AppProcessTest {
    private AppProcessTest() {
    }

    public static void main(String[] args) {
        System.load("/data/local/tmp/frida-java-bridge-rs/libfrida_java_bridge_rs.so");

        String result = nativeRun(AppProcessTest.class.getClassLoader());
        if (!"ok".equals(result)) {
            throw new RuntimeException(result);
        }

        System.out.println("app_process_test: ok");
        Runtime.getRuntime().halt(0);
    }

    private static native String nativeRun(ClassLoader loader);
}
