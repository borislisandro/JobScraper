import {describe,expect,it} from "vitest";
import {documentMetadataHeader,MAX_ATTACHMENT_BYTES,validateAttachmentSize} from "./api";

describe("document binary IPC",()=>{
 it("encodes Unicode metadata as unpadded base64url",()=>{
  const header=documentMetadataHeader({applicationId:"app-1",documentType:"resume",filename:"currículo_日本語.pdf",mimeType:"application/pdf"});
  expect(header).not.toMatch(/[+/=]/);
  const decoded=JSON.parse(Buffer.from(header,"base64url").toString("utf8"));
  expect(decoded.filename).toBe("currículo_日本語.pdf");
 });
 it("accepts exactly 20 MB and rejects the next byte",()=>{expect(MAX_ATTACHMENT_BYTES).toBe(20*1024*1024);expect(()=>validateAttachmentSize(MAX_ATTACHMENT_BYTES)).not.toThrow();expect(()=>validateAttachmentSize(MAX_ATTACHMENT_BYTES+1)).toThrow("Attachment exceeds 20 MB")});
});
