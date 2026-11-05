package dev.fetchx.durabilityfixture;

import android.content.Context;
import android.os.Process;
import android.os.SystemClock;
import android.util.AtomicFile;
import android.util.Log;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

final class AtomicSnapshotSlots {
    private static final String TAG = "TabDurability";
    private static final int MAGIC = 0x46585453;
    private static final int FORMAT_VERSION = 1;
    private static final int MAX_TABS = 100;
    private static final int CHECKSUM_BYTES = 32;

    private final Context context;
    private final File directory;

    AtomicSnapshotSlots(Context context, String runId) {
        if (runId == null || !runId.matches("[A-Za-z0-9_-]{1,48}")) {
            throw new IllegalArgumentException("Invalid fixture run ID");
        }
        this.context = context.getApplicationContext();
        this.directory = new File(context.getFilesDir(), "atomic-snapshot-fixture-" + runId);
    }

    void write(String runId, long generation, String operation, int delayMs, int tabCount) {
        try {
            long encodeStarted = SystemClock.elapsedRealtimeNanos();
            Snapshot snapshot = createSnapshot(runId, generation, tabCount);
            byte[] encoded = encode(snapshot);
            long encodeNanos = SystemClock.elapsedRealtimeNanos() - encodeStarted;
            int slot = (int) (generation & 1);
            AtomicFile atomicFile = atomicFile(slot);
            writeReceipt(runId, generation, operation);
            long writeStarted = SystemClock.elapsedRealtimeNanos();
            if ("partial".equals(operation)) {
                FileOutputStream stream = atomicFile.startWrite();
                stream.write(encoded, 0, Math.max(1, encoded.length / 2));
                stream.flush();
                writeMarker("partial-" + runId, "generation=" + generation);
                SystemClock.sleep(10_000);
                atomicFile.failWrite(stream);
            } else if ("fail".equals(operation)) {
                FileOutputStream stream = atomicFile.startWrite();
                stream.write(encoded);
                atomicFile.failWrite(stream);
            } else {
                FileOutputStream stream = atomicFile.startWrite();
                stream.write(encoded);
                atomicFile.finishWrite(stream);
            }
            long writeNanos = SystemClock.elapsedRealtimeNanos() - writeStarted;
            writeMarker("metrics-" + runId,
                    "generation=" + generation + ";tabs=" + snapshot.ids.size()
                            + ";bytes=" + encoded.length + ";encodeNanos=" + encodeNanos
                            + ";writeNanos=" + writeNanos);
            Log.i(TAG, "ATOMIC_WRITE run=" + runId + " generation=" + generation
                    + " tabs=" + snapshot.ids.size() + " bytes=" + encoded.length
                    + " encodeMs=" + (encodeNanos / 1_000_000.0)
                    + " writeMs=" + (writeNanos / 1_000_000.0)
                    + " operation=" + operation + " delayMs=" + delayMs);
            SystemClock.sleep(delayMs);
            Process.killProcess(Process.myPid());
        } catch (IOException error) {
            Log.e(TAG, "Atomic fixture write failed for " + runId, error);
            writeMarker("failure-" + runId, error.toString());
            Process.killProcess(Process.myPid());
        }
    }

    void read(String runId) {
        long started = SystemClock.elapsedRealtimeNanos();
        Runtime runtime = Runtime.getRuntime();
        long heapBefore = runtime.totalMemory() - runtime.freeMemory();
        Snapshot latest = null;
        try {
            for (int slot = 0; slot < 2; slot++) {
                byte[] bytes = readSlot(slot);
                if (bytes == null) continue;
                Snapshot candidate = decode(bytes);
                if (candidate != null && (latest == null || candidate.generation > latest.generation)) {
                    latest = candidate;
                }
            }
            long readNanos = SystemClock.elapsedRealtimeNanos() - started;
            long heapAfter = runtime.totalMemory() - runtime.freeMemory();
            String result = latest == null ? "run=" + runId + ";valid=false;generation=0"
                    : "run=" + runId + ";valid=true;generation=" + latest.generation
                    + ";tabs=" + latest.ids.size() + ";active=" + latest.activeId
                    + ";snapshotRun=" + latest.runId
                    + ";readNanos=" + readNanos + ";heapDeltaBytes=" + (heapAfter - heapBefore);
            writeMarker("read-" + runId, result);
            Log.i(TAG, "ATOMIC_READ " + result);
        } catch (IOException error) {
            Log.e(TAG, "Atomic fixture read failed for " + runId, error);
            writeMarker("read-" + runId, "run=" + runId + ";error=" + error);
        }
        Process.killProcess(Process.myPid());
    }

    void corruptNewest(String runId) {
        try {
            Snapshot latest = latestSnapshot();
            if (latest == null) throw new IOException("No valid snapshot to corrupt");
            File base = baseFile((int) (latest.generation & 1));
            try (FileOutputStream stream = new FileOutputStream(base, false)) {
                stream.write(new byte[] {1, 2, 3});
                stream.getFD().sync();
            }
            writeMarker("corrupt-" + runId, "generation=" + latest.generation);
            Log.i(TAG, "ATOMIC_CORRUPT run=" + runId + " generation=" + latest.generation);
        } catch (IOException error) {
            Log.e(TAG, "Could not corrupt isolated fixture snapshot", error);
            writeMarker("corrupt-" + runId, "error=" + error);
        }
        Process.killProcess(Process.myPid());
    }

    private Snapshot latestSnapshot() throws IOException {
        Snapshot latest = null;
        for (int slot = 0; slot < 2; slot++) {
            byte[] bytes = readSlot(slot);
            if (bytes == null) continue;
            Snapshot candidate = decode(bytes);
            if (candidate != null && (latest == null || candidate.generation > latest.generation)) {
                latest = candidate;
            }
        }
        return latest;
    }

    private byte[] readSlot(int slot) throws IOException {
        File base = baseFile(slot);
        if (!base.exists() && !new File(base.getPath() + ".bak").exists()) return null;
        try (FileInputStream stream = atomicFile(slot).openRead();
             ByteArrayOutputStream output = new ByteArrayOutputStream()) {
            byte[] buffer = new byte[4096];
            int read;
            while ((read = stream.read(buffer)) != -1) {
                output.write(buffer, 0, read);
                if (output.size() > 1_048_576) throw new IOException("Snapshot exceeds size limit");
            }
            return output.toByteArray();
        }
    }

    private AtomicFile atomicFile(int slot) throws IOException {
        if (!directory.exists() && !directory.mkdirs()) throw new IOException("Cannot create fixture directory");
        return new AtomicFile(baseFile(slot));
    }

    private File baseFile(int slot) {
        return new File(directory, "snapshot-" + slot);
    }

    private Snapshot createSnapshot(String runId, long generation, int requestedTabCount) {
        int count = requestedTabCount > 0
                ? Math.min(requestedTabCount, MAX_TABS)
                : generation == 1 ? 4 : generation == 2 ? 3 : generation == 3 ? 2 : 1;
        List<String> ids = new ArrayList<>();
        List<String> urls = new ArrayList<>();
        for (int index = 1; index <= count; index++) {
            ids.add(runId + "-g" + generation + "-tab" + index);
            urls.add("https://fixture.invalid/" + runId + "/" + generation + "/" + index);
        }
        return new Snapshot(runId, generation, ids, urls, ids.get(ids.size() - 1));
    }

    private byte[] encode(Snapshot snapshot) throws IOException {
        ByteArrayOutputStream payloadBuffer = new ByteArrayOutputStream();
        try (DataOutputStream payload = new DataOutputStream(payloadBuffer)) {
            payload.writeInt(MAGIC);
            payload.writeInt(FORMAT_VERSION);
            payload.writeLong(snapshot.generation);
            payload.writeUTF(snapshot.runId);
            payload.writeInt(snapshot.ids.size());
            for (int index = 0; index < snapshot.ids.size(); index++) {
                payload.writeUTF(snapshot.ids.get(index));
                payload.writeUTF(snapshot.urls.get(index));
            }
            payload.writeUTF(snapshot.activeId);
        }
        byte[] bytes = payloadBuffer.toByteArray();
        ByteArrayOutputStream envelope = new ByteArrayOutputStream(bytes.length + CHECKSUM_BYTES);
        envelope.write(bytes);
        envelope.write(sha256(bytes));
        return envelope.toByteArray();
    }

    private Snapshot decode(byte[] envelope) {
        if (envelope.length <= CHECKSUM_BYTES) return null;
        int payloadLength = envelope.length - CHECKSUM_BYTES;
        byte[] payload = new byte[payloadLength];
        byte[] checksum = new byte[CHECKSUM_BYTES];
        System.arraycopy(envelope, 0, payload, 0, payloadLength);
        System.arraycopy(envelope, payloadLength, checksum, 0, CHECKSUM_BYTES);
        if (!MessageDigest.isEqual(sha256(payload), checksum)) return null;

        try (DataInputStream input = new DataInputStream(new ByteArrayInputStream(payload))) {
            if (input.readInt() != MAGIC || input.readInt() != FORMAT_VERSION) return null;
            long generation = input.readLong();
            String runId = input.readUTF();
            int count = input.readInt();
            if (generation <= 0 || count <= 0 || count > MAX_TABS) return null;
            List<String> ids = new ArrayList<>();
            List<String> urls = new ArrayList<>();
            Set<String> uniqueIds = new HashSet<>();
            for (int index = 0; index < count; index++) {
                String id = input.readUTF();
                String url = input.readUTF();
                if (id.isEmpty() || !uniqueIds.add(id)) return null;
                ids.add(id);
                urls.add(url);
            }
            String activeId = input.readUTF();
            if (!uniqueIds.contains(activeId) || input.available() != 0) return null;
            return new Snapshot(runId, generation, ids, urls, activeId);
        } catch (IOException error) {
            return null;
        }
    }

    private byte[] sha256(byte[] input) {
        try {
            return MessageDigest.getInstance("SHA-256").digest(input);
        } catch (NoSuchAlgorithmException error) {
            throw new IllegalStateException("SHA-256 is required by Android", error);
        }
    }

    private void writeReceipt(String runId, long generation, String operation) throws IOException {
        writeMarker("receipt-" + runId, "run=" + runId + ";generation=" + generation + ";operation=" + operation);
    }

    private void writeMarker(String name, String value) {
        try (FileOutputStream output = context.openFileOutput(name, Context.MODE_PRIVATE)) {
            output.write(value.getBytes(StandardCharsets.UTF_8));
            output.getFD().sync();
        } catch (IOException error) {
            Log.e(TAG, "Could not write fixture marker " + name, error);
        }
    }

    private static final class Snapshot {
        final String runId;
        final long generation;
        final List<String> ids;
        final List<String> urls;
        final String activeId;

        Snapshot(String runId, long generation, List<String> ids, List<String> urls, String activeId) {
            this.runId = runId;
            this.generation = generation;
            this.ids = ids;
            this.urls = urls;
            this.activeId = activeId;
        }
    }
}
