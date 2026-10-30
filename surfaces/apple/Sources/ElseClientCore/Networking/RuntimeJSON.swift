import Foundation

public enum RuntimeJSON {
    public static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let container = try decoder.singleValueContainer()
            let value = try container.decode(String.self)

            if let date = try? Date.ISO8601FormatStyle(
                includingFractionalSeconds: true
            ).parse(value) {
                return date
            }
            if let date = try? Date.ISO8601FormatStyle().parse(value) {
                return date
            }
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "Expected an RFC 3339 date: \(value)"
            )
        }
        return decoder
    }

    public static func encoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .custom { date, encoder in
            var container = encoder.singleValueContainer()
            let value = date.formatted(
                Date.ISO8601FormatStyle(includingFractionalSeconds: true)
            )
            try container.encode(value)
        }
        return encoder
    }
}
