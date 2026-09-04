package com.deepscan.parser;

import io.grpc.Server;
import io.grpc.ServerBuilder;
import org.apache.tika.metadata.Metadata;
import org.apache.tika.parser.AutoDetectParser;
import org.apache.tika.parser.ParseContext;
import org.apache.tika.sax.BodyContentHandler;

import java.io.FileInputStream;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Small always-on JVM service exposing ParserBridgeService (see
 * proto/deepscan.proto) to the Rust engine over loopback gRPC. Every
 * document/enterprise-format extraction in DeepScan's pipeline (see
 * docs/ARCHITECTURE.md #1) routes through this process; nothing here ever
 * leaves the machine.
 */
public final class TikaServer {

    private static final int PORT = 51500; // fixed port; written to parser.lock alongside engine.lock

    public static void main(String[] args) throws Exception {
        Server server = ServerBuilder.forPort(PORT)
                .addService(new ParserBridgeServiceImpl())
                .build()
                .start();

        writeLockfile();
        System.out.println("DeepScan parser bridge listening on 127.0.0.1:" + PORT);
        server.awaitTermination();
    }

    private static void writeLockfile() throws Exception {
        String home = System.getProperty("user.home");
        Path lockPath = Path.of(home, ".deepscan", "parser.lock");
        Files.createDirectories(lockPath.getParent());
        Files.writeString(lockPath, "{\"port\": " + PORT + "}");
    }

    /**
     * Runs Tika's AutoDetectParser against a file and returns extracted text
     * + metadata. Falls back note: if the parsed body is empty for a PDF
     * (scanned, no text layer), the Rust engine's `ocrs` OCR path picks up
     * the slack rather than this service — see docs/ARCHITECTURE.md.
     */
    static String extractText(String path) throws Exception {
        try (InputStream stream = new FileInputStream(path)) {
            AutoDetectParser parser = new AutoDetectParser();
            BodyContentHandler handler = new BodyContentHandler(-1); // no size cap
            Metadata metadata = new Metadata();
            parser.parse(stream, handler, metadata, new ParseContext());
            return handler.toString();
        }
    }
}
