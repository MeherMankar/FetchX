package dev.fetchx.durabilityfixture;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.os.Process;
import android.os.SystemClock;
import android.util.Log;

import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;

public final class FixtureReceiver extends BroadcastReceiver {
    private static final String TAG = "TabDurability";
    private static final String PREFS = "tab_persistence_durability_isolated";
    private static final String TABS_KEY = "saved_browser_tabs";
    private static final String ACTIVE_KEY = "saved_active_tab_id";
    private static final String VERSION_KEY = "snapshot_version";
    private static final String READ_RESULT_KEY = "read_verification";

    @Override
    public void onReceive(Context context, Intent intent) {
        String mode = intent.getStringExtra("mode");
        String runId = intent.getStringExtra("runId");
        if ("write".equals(mode)) {
            writeSnapshots(context, runId, intent.getStringExtra("method"),
                    intent.getIntExtra("delayMs", 0),
                    intent.getIntExtra("version", 4),
                    intent.getBooleanExtra("measure", false));
        } else if ("read".equals(mode)) {
            readSnapshot(context, runId, intent.getIntExtra("expectedVersion", 4));
        } else if ("atomicWrite".equals(mode)) {
            new AtomicSnapshotSlots(context, runId).write(
                    runId,
                    intent.getLongExtra("generation", 1),
                    intent.getStringExtra("operation"),
                    intent.getIntExtra("delayMs", 0),
                    intent.getIntExtra("tabCount", 0));
        } else if ("atomicRead".equals(mode)) {
            new AtomicSnapshotSlots(context, runId).read(runId);
        } else if ("atomicCorrupt".equals(mode)) {
            new AtomicSnapshotSlots(context, runId).corruptNewest(runId);
        } else {
            reportAndExit("FAIL", runId, "missing or invalid mode");
        }
    }

    private void writeSnapshots(Context context, String runId, String method, int delayMs,
            int version, boolean measure) {
        if (runId == null || method == null || (!"apply".equals(method) && !"commit".equals(method))
                || version < 1 || version > 4) {
            reportAndExit("FAIL", runId, "invalid write arguments");
            return;
        }

        SharedPreferences preferences = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        String tabs = encodeTabs(runId, version);
        String activeId = activeIdFor(runId, version);
        if (!writeReceipt(context, runId, version, method)) {
            reportAndExit("FAIL", runId, "could not record writer receipt");
            return;
        }
        SharedPreferences.Editor editor = preferences.edit()
                .putString(TABS_KEY, encodeSnapshot(runId, version, tabs, activeId))
                .putString(ACTIVE_KEY, activeId)
                .putInt(VERSION_KEY, version);
        long started = SystemClock.elapsedRealtimeNanos();
        if ("commit".equals(method)) {
            if (!editor.commit()) {
                reportAndExit("FAIL", runId, "commit returned false at version " + version);
                return;
            }
        } else {
            editor.apply();
        }
        long writeDurationNanos = SystemClock.elapsedRealtimeNanos() - started;
        if (!matches(preferences, runId, version)) {
            reportAndExit("FAIL", runId, "same-process snapshot mismatch at version " + version);
            return;
        }

        if (measure && !writeMetrics(context, runId, version, method, writeDurationNanos)) {
            reportAndExit("FAIL", runId, "could not record write timing");
            return;
        }
        Log.i(TAG, "WRITE run=" + runId + " version=" + version + " method=" + method
                + " delayMs=" + delayMs + " writeMs=" + (writeDurationNanos / 1_000_000.0));
        SystemClock.sleep(delayMs);
        Process.killProcess(Process.myPid());
    }

    private boolean writeReceipt(Context context, String runId, int version, String method) {
        String receipt = "run=" + runId + ";version=" + version + ";method=" + method;
        try (FileOutputStream output = context.openFileOutput(
                "receipt-" + runId + ".txt", Context.MODE_PRIVATE)) {
            output.write(receipt.getBytes(StandardCharsets.UTF_8));
            output.getFD().sync();
            return true;
        } catch (IOException exception) {
            Log.e(TAG, "Unable to write receipt for " + runId, exception);
            return false;
        }
    }

    private boolean writeMetrics(Context context, String runId, int version, String method, long durationNanos) {
        String metrics = "run=" + runId + ";version=" + version + ";method=" + method
                + ";durationNanos=" + durationNanos;
        try (FileOutputStream output = context.openFileOutput(
                "metrics-" + runId + ".txt", Context.MODE_PRIVATE)) {
            output.write(metrics.getBytes(StandardCharsets.UTF_8));
            output.getFD().sync();
            return true;
        } catch (IOException exception) {
            Log.e(TAG, "Unable to write timing record for " + runId, exception);
            return false;
        }
    }

    private void readSnapshot(Context context, String runId, int expectedVersion) {
        SharedPreferences preferences = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        boolean valid = matches(preferences, runId, expectedVersion);
        String tabs = preferences.getString(TABS_KEY, null);
        String activeId = preferences.getString(ACTIVE_KEY, null);
        int actualVersion = preferences.getInt(VERSION_KEY, -1);
        String result = "run=" + runId + ";expected=" + expectedVersion
                + ";actual=" + actualVersion + ";valid=" + valid;
        boolean recorded = preferences.edit().putString(READ_RESULT_KEY, result).commit();
        Log.i(TAG, "READ run=" + runId + " expectedVersion=" + expectedVersion
                + " actualVersion=" + actualVersion + " valid=" + valid
                + " recorded=" + recorded + " tabs=" + tabs + " active=" + activeId);
        Process.killProcess(Process.myPid());
    }

    private boolean matches(SharedPreferences preferences, String runId, int version) {
        String tabs = preferences.getString(TABS_KEY, null);
        String activeId = preferences.getString(ACTIVE_KEY, null);
        return preferences.getInt(VERSION_KEY, -1) == version
                && tabs != null
                && tabs.equals(encodeSnapshot(runId, version, encodeTabs(runId, version), activeIdFor(runId, version)))
                && activeId != null
                && activeId.equals(activeIdFor(runId, version));
    }

    private String encodeTabs(String runId, int version) {
        int count = version == 1 ? 4 : version == 2 ? 3 : version == 3 ? 2 : 1;
        StringBuilder tabs = new StringBuilder();
        for (int index = 1; index <= count; index++) {
            if (index > 1) {
                tabs.append(',');
            }
            tabs.append(tabId(runId, version, index));
        }
        return tabs.toString();
    }

    private String activeIdFor(String runId, int version) {
        int index = version == 1 ? 4 : version == 2 ? 3 : version == 3 ? 2 : 1;
        return tabId(runId, version, index);
    }

    private String tabId(String runId, int version, int index) {
        return runId + "-v" + version + "-tab" + index;
    }

    private String encodeSnapshot(String runId, int version, String tabs, String activeId) {
        return "run=" + runId + ";version=" + version + ";tabs=" + tabs + ";active=" + activeId;
    }

    private void reportAndExit(String result, String runId, String detail) {
        Log.i(TAG, result + " run=" + runId + " detail=" + detail);
        Process.killProcess(Process.myPid());
    }
}
