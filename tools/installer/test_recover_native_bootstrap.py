import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import recover_native_bootstrap as recovery
from couch_install import InstallError


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.root.chmod(0o700)
        self.source = self.root/'failed'; self.source.mkdir(mode=0o700)
        ordered = sorted(recovery.ORIGINALS)+['userdata']
        self.offsets = {name:(i+1)*512 for i,name in enumerate(ordered)}
        partitions = {name:{'offset':offset,'size':512} for name,offset in self.offsets.items()}
        partitions['flashinfo']={'offset':(len(ordered)+1)*512,'size':512}
        originals={}
        for index,name in enumerate(sorted(recovery.ORIGINALS)):
            data=bytes([index+1])*512; filename=f'bootstrap-{name}.img'
            (self.source/filename).write_bytes(data)
            originals[name]={'file':filename,'size':512,'sha256':recovery.sha(data)}
        self.record={'schema':1,'kind':'couch-device-enrollment','model':recovery.MODEL,'original_os':'Couch',
                     'cid':'1'*32,'capacity':(len(ordered)+2)*512,'partitions':partitions,'originals':originals,
                     'identity_sha256':{n:originals[n]['sha256'] for n in recovery.IDENTITY_PARTITIONS}}
        raw=json.dumps(self.record).encode();(self.source/'current-couch-snapshot.json').write_bytes(raw)
        self.stage=recovery.sha(b'temporary boot')
        self.events=[]
        self.event('created','created',{})
        self.event('inputs_verified','transition',{'event':'inputs_verified'})
        for name,item in sorted(originals.items()):self.event('inputs_verified','checkpoint',{'event':'bootstrap_original_verified','target':name,**item})
        self.event('android_bound','transition',{'event':'retained_enrollment_bound','original_os':'Couch','cid':self.record['cid']})
        self.event('originals_saved','transition',{'event':'enrollment_complete','enrollment_sha256':recovery.sha(raw)})
        self.event('stage_boot_pending','transition',{'event':'bootstrap_write_admitted','stage_sha256':self.stage})
        self.event('stage_boot_pending','checkpoint',{'event':'bootstrap_readback_verified','stage_sha256':self.stage})
        self.event('failed','transition',{'event':'installation_stopped'})
        self.pin=patch.object(recovery,'official_inputs',return_value=({},self.offsets));self.pin.start();self.addCleanup(self.pin.stop)

    def event(self,phase,kind,evidence):
        i=len(self.events);value={'schema':1,'sequence':i,'phase':phase,'kind':kind,'evidence':evidence};self.events.append(value)
        (self.source/f'event-{i:05}.json').write_text(json.dumps(value))

    def admit(self):return recovery.admit(self.source,self.stage)

    def device(self):
        record=self.record
        class Fake:
            def __init__(self):
                self.description={'hwcode':0x6580,'cid_encoding':'mt6580-legacy-le32-registers',
                                  'runtime_cid_sha256':recovery.sha(bytes.fromhex(record['cid'])),
                                  'storage_id':'a'*64,'capacity':record['capacity'],'partitions':copy.deepcopy(record['partitions'])}
                self.hashes={n:x['sha256'] for n,x in record['originals'].items()};self.writes=[];self.closed=False
            def hash(self,name):return self.hashes[name]
            def write(self,name,path):self.writes.append(name);self.hashes[name]=recovery.sha(Path(path).read_bytes())
            def close(self):self.closed=True
        device=Fake();device.hashes['boot']=self.stage;return device

    def test_exact_evidence_restores_only_original_boot_and_retains_old_session(self):
        proof=self.admit();device=self.device();before={p.name:p.read_bytes() for p in self.source.iterdir()}
        def factory(release,bundle,binding):
            self.assertEqual(set(release['images']),{'boot'});self.assertEqual(bundle,self.source)
            self.assertEqual(binding['partitions'],self.record['partitions']);return device
        result=recovery.recover(proof,device,factory,self.root/'recovery')
        self.assertEqual(result,{'restored':['boot'],'verified':True,'restart_requested':False})
        self.assertEqual(device.writes,['boot']);self.assertTrue(device.closed)
        self.assertTrue(json.loads((self.root/'recovery/verified.json').read_text())['complete'])
        self.assertEqual(before,{p.name:p.read_bytes() for p in self.source.iterdir()})

    def test_wrong_live_cid_boot_or_retained_partition_never_constructs_writer(self):
        proof=self.admit()
        for kind in ('cid','boot','recovery','odmdtbo','nvram'):
            with self.subTest(kind=kind):
                device=self.device()
                if kind=='cid':device.description['runtime_cid_sha256']='f'*64
                else:device.hashes[kind]='f'*64
                with self.assertRaises(InstallError):recovery.recover(proof,device,lambda *args:self.fail('Writer constructed'),self.root/('denied-'+kind))
                self.assertFalse((self.root/('denied-'+kind)).exists());self.assertEqual(device.writes,[])

    def test_changed_backup_and_missing_readback_checkpoint_refuse_admission(self):
        path=self.source/'bootstrap-boot.img';old=path.read_bytes();path.write_bytes(b'x'*512)
        with self.assertRaises(InstallError):self.admit()
        path.write_bytes(old)
        item=self.events[-2];item['evidence']['event']='not_readback'
        (self.source/f"event-{item['sequence']:05}.json").write_text(json.dumps(item))
        with self.assertRaises(InstallError):self.admit()

    def test_later_install_phase_or_wrong_stage_pin_cannot_be_recovered(self):
        with self.assertRaises(InstallError):recovery.admit(self.source,'f'*64)
        event=self.events[-1];event['phase']='stage_bound'
        (self.source/f"event-{event['sequence']:05}.json").write_text(json.dumps(event))
        with self.assertRaises(InstallError):self.admit()

    def test_bad_restore_readback_never_publishes_success_or_retries(self):
        proof=self.admit();device=self.device()
        def bad_write(name,path):device.writes.append(name);device.hashes[name]='e'*64
        device.write=bad_write
        with self.assertRaises(InstallError):recovery.recover(proof,device,lambda *args:device,self.root/'recovery')
        self.assertEqual(device.writes,['boot']);self.assertTrue(device.closed)
        self.assertFalse((self.root/'recovery/verified.json').exists())

    def test_symlink_original_and_replaced_evidence_are_rejected(self):
        proof=self.admit();target=self.source/'bootstrap-logo.img';copyfile=self.root/'outside';copyfile.write_bytes(target.read_bytes())
        target.unlink();target.symlink_to(copyfile)
        with self.assertRaises(InstallError):self.admit()
        device=self.device()
        with self.assertRaises(InstallError):recovery.recover(proof,device,lambda *args:self.fail('Writer constructed'),self.root/'denied')

if __name__=='__main__':unittest.main()
