# metrics-exporter-scope communication protocol

The metrics-exporter-scope communication protocol is a TCP protocol to exchange
metrics between the server process and clients.

## Defaults

The default port is `5001`.

## Data serialization

The serialization format is a MessagePack.

## Chat

* After the connection is established, the server writes 2-byte VERSION packet,
  which contains the protocol version, encoded in little-endian. If the client
  does not support the protocol version, it should close the connection.

* The client sends serialized `ClientSettings` structure:
```json
{
  "sampling_interval": 1000000
}
```

where

* `sampling_interval` is the interval the server thread should sample the
  metrics and send them to the client. The interval is specified in
  nanoseconds.

## Communication

The server sends serialized metrics snapshot packets as well as information
ones to the client. The first packet is always an information one. The client
should determine the packet type according to its structure.

### Information packets

The information packets are used to send metrics metadata to the client. The
server sends such packets every 5 seconds (by default).

```json
{
    "metrics": {
        "metric_name": {
            "labels": {
                "label_name": "label_value",
                "label_name2": "label_value2"
            }
        },
        "metric_name2": {
            "labels": {
                "label_name": "label_value",
                "label_name2": "label_value2"
            }
        }
    }
}
```

The client may use metrics labels as hints for displaying the data. The default
labels are:

* `plot` group several metrics into a single plot

* `color` specify the color of the line in the plot

### Snapshot packets

The snapshot packets contain the actual metrics data. The server sends such
using the sampling interval, specified during `Chat` phase.

```json
{
    "t": 1234567890,
    "d": {
        "metric_name": 123.4,
        "metric_name2": 456.7
    }
}
```

where

* `t` is the timestamp of the snapshot, in nanoseconds. The timestamp is monotonic
  and relative to the time point the `Communication` phase started at.

* `d` is the dictionary of metrics. The keys are metric names, and the values
  are float numbers.

The payload always contains state of all metrics at the moment of the snapshot,
despite the metrics have been changed or not.
