package frida.rust.java.bridge.performtest;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;

import java.io.File;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;

public final class EarlyPerformStatusProvider extends ContentProvider {
    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public Bundle call(String method, String arg, Bundle extras) {
        Bundle result = new Bundle();
        if (!"status".equals(method) && !"reset".equals(method)) {
            result.putString("status", "error: unsupported method " + method);
            return result;
        }

        File file = new File(getContext().getFilesDir(), "apk-perform-status.txt");
        if ("reset".equals(method)) {
            if (!file.exists() || file.delete()) {
                result.putString("status", "reset");
            } else {
                result.putString("status", "error: unable to delete status file");
            }
            return result;
        }

        if (!file.exists()) {
            result.putString("status", "missing");
            return result;
        }

        try {
            String status = new String(Files.readAllBytes(file.toPath()), StandardCharsets.UTF_8);
            result.putString("status", status.trim());
        } catch (IOException e) {
            result.putString("status", "error: " + e);
        }
        return result;
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs, String sortOrder) {
        return null;
    }

    @Override
    public String getType(Uri uri) {
        return null;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        return null;
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        return 0;
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        return 0;
    }
}
