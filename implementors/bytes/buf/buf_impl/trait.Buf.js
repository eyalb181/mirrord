(function() {var implementors = {
"aws_smithy_http":[["impl <a class=\"trait\" href=\"bytes/buf/buf_impl/trait.Buf.html\" title=\"trait bytes::buf::buf_impl::Buf\">Buf</a> for <a class=\"struct\" href=\"aws_smithy_http/byte_stream/struct.AggregatedBytes.html\" title=\"struct aws_smithy_http::byte_stream::AggregatedBytes\">AggregatedBytes</a>"]],
"bytes":[],
"bytes_utils":[["impl&lt;B: <a class=\"trait\" href=\"bytes/buf/buf_impl/trait.Buf.html\" title=\"trait bytes::buf::buf_impl::Buf\">Buf</a>&gt; <a class=\"trait\" href=\"bytes/buf/buf_impl/trait.Buf.html\" title=\"trait bytes::buf::buf_impl::Buf\">Buf</a> for <a class=\"struct\" href=\"bytes_utils/struct.SegmentedBuf.html\" title=\"struct bytes_utils::SegmentedBuf\">SegmentedBuf</a>&lt;B&gt;"],["impl&lt;'a, B: <a class=\"trait\" href=\"bytes/buf/buf_impl/trait.Buf.html\" title=\"trait bytes::buf::buf_impl::Buf\">Buf</a>&gt; <a class=\"trait\" href=\"bytes/buf/buf_impl/trait.Buf.html\" title=\"trait bytes::buf::buf_impl::Buf\">Buf</a> for <a class=\"struct\" href=\"bytes_utils/struct.SegmentedSlice.html\" title=\"struct bytes_utils::SegmentedSlice\">SegmentedSlice</a>&lt;'a, B&gt;"]],
"hyper":[],
"tonic":[["impl <a class=\"trait\" href=\"bytes/buf/buf_impl/trait.Buf.html\" title=\"trait bytes::buf::buf_impl::Buf\">Buf</a> for <a class=\"struct\" href=\"tonic/codec/struct.DecodeBuf.html\" title=\"struct tonic::codec::DecodeBuf\">DecodeBuf</a>&lt;'_&gt;"]],
"tungstenite":[["impl&lt;const CHUNK_SIZE: <a class=\"primitive\" href=\"https://doc.rust-lang.org/nightly/std/primitive.usize.html\">usize</a>&gt; <a class=\"trait\" href=\"bytes/buf/buf_impl/trait.Buf.html\" title=\"trait bytes::buf::buf_impl::Buf\">Buf</a> for <a class=\"struct\" href=\"tungstenite/buffer/struct.ReadBuffer.html\" title=\"struct tungstenite::buffer::ReadBuffer\">ReadBuffer</a>&lt;CHUNK_SIZE&gt;"]]
};if (window.register_implementors) {window.register_implementors(implementors);} else {window.pending_implementors = implementors;}})()