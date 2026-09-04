package com.deepscan.parser;

import com.deepscan.parser.pb.ExtractRequest;
import com.deepscan.parser.pb.ExtractResponse;
import com.deepscan.parser.pb.ParserBridgeServiceGrpc;
import io.grpc.stub.StreamObserver;

/** gRPC-generated stub implementation — thin wrapper around {@link TikaServer#extractText}. */
final class ParserBridgeServiceImpl extends ParserBridgeServiceGrpc.ParserBridgeServiceImplBase {

    @Override
    public void extractDocument(ExtractRequest request, StreamObserver<ExtractResponse> responseObserver) {
        try {
            String text = TikaServer.extractText(request.getPath());
            ExtractResponse response = ExtractResponse.newBuilder()
                    .setExtractedText(text)
                    .setOcrUsed(false)
                    .build();
            responseObserver.onNext(response);
            responseObserver.onCompleted();
        } catch (Exception e) {
            responseObserver.onError(e);
        }
    }
}
